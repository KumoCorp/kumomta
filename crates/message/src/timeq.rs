use crate::message::{MessageList, MessageWithIdAdapter};
use crate::Message;
use intrusive_collections::LinkedList;
use kumo_chrono_helper::{DateTime, Utc};
use tokio::time::{Duration, Instant};

const WHEEL_BITS: usize = 8;
const WHEEL_SIZE: usize = 256;
const WHEEL_MASK: usize = WHEEL_SIZE - 1;

/// A time ordered queue of Messages
pub struct TimeQ<const SLOTS: usize = 4> {
    tick_resolution: Duration,
    created: Instant,
    next_run: Instant,
    last_dispatched: Instant,
    len: usize,
    buckets: [Bucket; SLOTS],
}

pub type QuadTimeQ = TimeQ<4>;

struct Bucket {
    lists: [LinkedList<MessageWithIdAdapter>; WHEEL_SIZE],
}

impl Default for Bucket {
    fn default() -> Self {
        Self {
            lists: std::array::from_fn(|_| LinkedList::default()),
        }
    }
}

/// Helper trait to get a version of the number of milliseconds
/// in a Duration, but rounding up rather than down
trait RoundedMillis {
    fn as_millis_round_up(&self) -> u128;
}

impl RoundedMillis for Duration {
    fn as_millis_round_up(&self) -> u128 {
        self.as_micros().div_ceil(1000)
    }
}

#[derive(Copy, Clone)]
enum RoundDirection {
    Up,
    Down,
}

impl<const SLOTS: usize> TimeQ<SLOTS> {
    fn new_impl(now: Instant, tick_resolution: Duration) -> Self {
        assert!(SLOTS > 0 && SLOTS <= 4, "SLOTS must be 1..=4");
        Self {
            tick_resolution,
            next_run: now + tick_resolution,
            last_dispatched: now,
            created: now,
            len: 0,
            buckets: std::array::from_fn(|_| Default::default()),
        }
    }

    pub fn new(tick_resolution: Duration) -> Self {
        Self::new_impl(Instant::now(), tick_resolution)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn tick_resolution(&self) -> Duration {
        self.tick_resolution
    }

    /// Computes the slot offset that will hold the provided due instant,
    /// or None if it would be in the past
    fn compute_abs_tick(&self, due: Instant, round_direction: RoundDirection) -> Option<usize> {
        let delta = due.checked_duration_since(self.created)?;
        match round_direction {
            RoundDirection::Up => Some(
                (delta
                    .as_millis_round_up()
                    .div_ceil(self.tick_resolution.as_millis_round_up())) as usize,
            ),
            RoundDirection::Down => {
                Some((delta.as_millis() / self.tick_resolution.as_millis()) as usize)
            }
        }
    }

    fn compute_list(
        &mut self,
        due: Instant,
        round_direction: RoundDirection,
    ) -> Option<&mut LinkedList<MessageWithIdAdapter>> {
        let next_run_tick = self.compute_abs_tick(self.next_run, round_direction)?;
        let mut due = self.compute_abs_tick(due, round_direction)?;
        let diff = due.checked_sub(next_run_tick)?;

        for slot in 0..SLOTS {
            let ceiling = 1 << (slot + 1) * WHEEL_BITS;
            if diff < ceiling {
                return self
                    .buckets
                    .get_mut(slot)
                    .unwrap()
                    .lists
                    .get_mut((due >> (slot * WHEEL_BITS)) & WHEEL_MASK);
            }
        }

        // In the largest slot
        if diff > std::u32::MAX as usize {
            due = next_run_tick + std::u32::MAX as usize
        }

        self.buckets
            .last_mut()
            .unwrap()
            .lists
            .get_mut((due >> (SLOTS - 1) * WHEEL_BITS) & WHEEL_MASK)
    }

    fn insert_impl(
        &mut self,
        now: Instant,
        now_chrono: DateTime<Utc>,
        message: Message,
        round_direction: RoundDirection,
    ) -> Result<(), Message> {
        let Some(due) = message.get_due() else {
            // It is due immediately, do not add here
            return Err(message);
        };

        let Ok(due_in) = (due - now_chrono).to_std() else {
            // Out of range, which implies that it is due now
            return Err(message);
        };

        if due_in <= Duration::ZERO {
            // Due either in the past, or due immediately
            return Err(message);
        }

        let due_instant = now + due_in;

        match self.compute_list(due_instant, round_direction) {
            Some(list) => {
                list.push_back(message.msg_and_id);
                self.len += 1;
                Ok(())
            }
            None => Err(message),
        }
    }

    /// Return all messages that are due at the specified now/now_utc.
    fn pop_impl(&mut self, now: Instant, now_utc: DateTime<Utc>) -> MessageList {
        let mut ready_messages = MessageList::new();

        if now < self.next_run {
            // We're not due to do anything
            return ready_messages;
        }

        let mut reinsert = LinkedList::default();

        // We are due (or perhaps over due); figure out which slot(s)
        // we need to process to get up to date
        let last_slot = self
            .compute_abs_tick(self.last_dispatched, RoundDirection::Down)
            .expect("never negative");
        let now_slot = self
            .compute_abs_tick(now, RoundDirection::Down)
            .expect("pop_impl failed because now is prior to the TimeQ creation");

        // Process all potential cascades first.
        // For the catch-up case we want to avoid making multiple
        // passes over tier-0 that we would otherwise need to make
        // if we put both things into the same loop
        for idx in last_slot + 1..=now_slot {
            if idx & WHEEL_MASK != 0 {
                continue;
            }
            // It is time to cascade timers

            /// Sweep all messages from bucket.lists[slot] into the reinsertion
            /// list, and return true if the next level should also cascade
            fn cascade(
                bucket: &mut Bucket,
                slot: usize,
                reinsert: &mut LinkedList<MessageWithIdAdapter>,
            ) -> bool {
                while let Some(msg_and_id) = bucket.lists[slot].pop_front() {
                    reinsert.push_back(msg_and_id);
                }
                slot == 0
            }

            for tier in 1..SLOTS {
                if !cascade(
                    &mut self.buckets[tier],
                    (idx >> (tier * WHEEL_BITS)) & WHEEL_MASK,
                    &mut reinsert,
                ) {
                    break;
                }
            }

            // Reinsert any messages that were promoted into the next
            // bucket, or collect any that are now ready into the ready list.
            // We round down when reinserting, so that we don't push out the
            // due time by an extra tick_resolution interval
            while let Some(msg_and_id) = reinsert.pop_front() {
                if let Err(msg) =
                    self.insert_impl(now, now_utc, Message { msg_and_id }, RoundDirection::Down)
                {
                    ready_messages.push_back(msg);
                } else {
                    // insert_impl incremented len, but we didn't
                    // really remove it as part of the cascade,
                    // so compensate for that now.
                    self.len -= 1;
                }
            }
        }

        // Constrain the number of passes over tier-0 to maximum of 1;
        // there's no sense visiting each tier-1 list slot more than once
        let num_slots = (now_slot - last_slot).min(WHEEL_SIZE);
        for idx in last_slot + 1..=last_slot + num_slots {
            // Retrieve any ready messages from the current slot
            let mut nominally_ready = self.buckets[0].lists[idx & WHEEL_MASK].take();
            while let Some(msg_and_id) = nominally_ready.pop_front() {
                if let Err(msg) =
                    self.insert_impl(now, now_utc, Message { msg_and_id }, RoundDirection::Down)
                {
                    ready_messages.push_back(msg);
                } else {
                    // insert_impl incremented len, but we didn't
                    // really remove it as part of the cascade,
                    // so compensate for that now.
                    self.len -= 1;
                }
            }
        }

        self.last_dispatched = now;
        self.next_run = now + self.tick_resolution;
        self.len -= ready_messages.len();

        ready_messages
    }

    /// Insert a message.
    /// If it is due immediately, Err(message) will be returned.
    pub fn insert(&mut self, message: Message) -> Result<(), Message> {
        // We round up when inserting so that very short near-future
        // intervals aren't immediately returned as ready
        self.insert_impl(Instant::now(), Utc::now(), message, RoundDirection::Up)
    }

    #[cfg(test)]
    fn insert_for_test(
        &mut self,
        message: Message,
        start: Instant,
        start_utc: DateTime<Utc>,
    ) -> Result<(), Message> {
        self.insert_impl(
            Instant::now(),
            start_utc + start.elapsed(),
            message,
            RoundDirection::Up,
        )
    }

    /// Pop all messages that are due now
    pub fn pop(&mut self) -> MessageList {
        self.pop_impl(Instant::now(), Utc::now())
    }

    /// Iterate the entire timeq and apply KEEPER to each item.
    /// If it returns true then the item will be retained in
    /// the timeq, otherwise, it will be unlinked from the timeq.
    pub fn retain<KEEPER>(&mut self, mut keeper: KEEPER)
    where
        KEEPER: FnMut(&Message) -> bool,
    {
        for bucket in self.buckets.iter_mut() {
            for list in bucket.lists.iter_mut() {
                let to_process = list.take();
                for msg_and_id in to_process {
                    let msg = Message { msg_and_id };
                    if (keeper)(&msg) {
                        // Keep it
                        list.push_back(msg.msg_and_id);
                    } else {
                        // Removed it
                        self.len -= 1;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EnvelopeAddress;
    use spool::SpoolId;
    use std::sync::Arc;

    #[derive(Debug)]
    struct Time {
        start: Instant,
        start_utc: DateTime<Utc>,
    }

    impl Time {
        pub fn new() -> Self {
            tokio::time::pause();
            let start_utc = Utc::now();
            let start = Instant::now();
            Self { start, start_utc }
        }

        pub fn elapsed(&self) -> Duration {
            self.start.elapsed()
        }

        pub async fn advance(&self, duration: Duration) {
            tokio::time::advance(duration).await;
        }

        pub fn now_utc(&self) -> DateTime<Utc> {
            self.start_utc + self.start.elapsed()
        }

        pub async fn new_msg_due_in(&self, duration: Duration) -> Message {
            let msg = new_msg();
            msg.set_due(Some(self.now_utc() + duration)).await.unwrap();
            msg
        }

        pub fn insert<const SLOTS: usize>(
            &self,
            timeq: &mut TimeQ<SLOTS>,
            msg: Message,
        ) -> Result<(), Message> {
            timeq.insert_for_test(msg, self.start, self.start_utc)
        }

        pub fn pop<const SLOTS: usize>(&self, timeq: &mut TimeQ<SLOTS>) -> MessageList {
            timeq.pop_impl(Instant::now(), self.now_utc())
        }

        pub async fn advance_and_collect<const SLOTS: usize>(
            &self,
            num_seconds: usize,
            timeq: &mut TimeQ<SLOTS>,
            popped: &mut Vec<Duration>,
        ) {
            for _ in 0..num_seconds {
                self.advance(Duration::from_secs(1)).await;
                let mut ready = self.pop(timeq);
                while let Some(_msg) = ready.pop_front() {
                    popped.push(self.start.elapsed());
                }
            }
        }
    }

    fn new_msg() -> Message {
        Message::new_dirty(
            SpoolId::new(),
            EnvelopeAddress::parse("sender@example.com").unwrap(),
            EnvelopeAddress::parse("recip@example.com").unwrap(),
            serde_json::json!({}),
            Arc::new("test".as_bytes().to_vec().into_boxed_slice()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn cannot_insert_immediately_due() {
        let mut timeq = QuadTimeQ::new(Duration::from_secs(3));
        assert!(timeq.is_empty());
        let msg1 = new_msg();
        assert!(timeq.insert(msg1).is_err());
        assert!(timeq.is_empty());
    }

    #[tokio::test]
    #[should_panic(expected = "attempted to insert an object that is already linked")]
    async fn double_insert() {
        let mut timeq = QuadTimeQ::new(Duration::from_secs(3));
        assert!(timeq.is_empty());
        let msg1 = new_msg();
        msg1.delay_by(chrono::Duration::seconds(10)).await.unwrap();
        assert!(timeq.insert(msg1.clone()).is_ok());
        assert!(timeq.insert(msg1).is_ok());
    }

    #[tokio::test]
    async fn retain() {
        let time = Time::new();

        let mut timeq = QuadTimeQ::new(Duration::from_secs(3));
        assert!(timeq.is_empty());

        let msg1 = time.new_msg_due_in(Duration::from_secs(10)).await;
        time.insert(&mut timeq, msg1.clone()).unwrap();
        assert_eq!(timeq.len(), 1);

        let msg2 = time.new_msg_due_in(Duration::from_secs(86400)).await;
        time.insert(&mut timeq, msg2.clone()).unwrap();
        assert_eq!(timeq.len(), 2);

        timeq.retain(|msg| *msg == msg2);
        assert_eq!(timeq.len(), 1);
    }

    async fn schedule_in_tier<const SLOTS: usize>(tier: usize) {
        let time = Time::new();

        let tick_resolution = Duration::from_secs(3);
        let mut base = tick_resolution;
        for _ in 0..tier {
            base *= WHEEL_SIZE as u32;
        }
        let limit = base * (WHEEL_SIZE as u32);
        eprintln!("max delay for tier {tier} is {limit:?}");

        let msg1 = time.new_msg_due_in(limit).await;

        eprintln!("schedule_in_tier: {time:?}");

        let mut timeq = TimeQ::<SLOTS>::new(tick_resolution);
        assert!(timeq.is_empty());

        eprintln!("msg is due: {:?}", msg1.get_due());
        time.insert(&mut timeq, msg1.clone()).unwrap();
        assert_eq!(timeq.len(), 1);

        assert!(time.pop(&mut timeq).is_empty());

        // We use binary partitioning of the overall limit time to
        // reduce the overall run time of the test, because at tier 2+
        // the exponentials are high and it will take forever for the
        // test to complete otherwise
        let mut wait = limit / 2;
        let mut ready_messages;
        loop {
            eprintln!("waiting for {wait:?}");
            time.advance(wait).await;
            wait = (wait / 2).max(tick_resolution);
            ready_messages = time.pop(&mut timeq);
            if !ready_messages.is_empty() {
                break;
            }
        }

        let elapsed = time.elapsed();
        let now_utc = time.now_utc();
        eprintln!("schedule_in_tier: {elapsed:?} {now_utc:?}");
        eprintln!("limit was {limit:?}, {elapsed:?} have elapsed");
        assert!(
            elapsed >= limit,
            "waited until {limit:?} had elapsed, but {elapsed:?} have elapsed",
        );
    }

    #[tokio::test]
    async fn quad_schedule_in_tier_0() {
        schedule_in_tier::<4>(0).await;
    }
    #[tokio::test]
    async fn quad_schedule_in_tier_1() {
        schedule_in_tier::<4>(1).await;
    }

    #[tokio::test]
    async fn quad_schedule_in_tier_2() {
        schedule_in_tier::<4>(2).await;
    }

    #[tokio::test]
    #[cfg(not(debug_assertions))]
    async fn quad_schedule_in_tier_3() {
        schedule_in_tier::<4>(3).await;
    }

    #[tokio::test]
    async fn tri_schedule_in_tier_0() {
        schedule_in_tier::<3>(0).await;
    }
    #[tokio::test]
    async fn tri_schedule_in_tier_1() {
        schedule_in_tier::<3>(1).await;
    }
    #[tokio::test]
    async fn tri_schedule_in_tier_2() {
        schedule_in_tier::<3>(2).await;
    }

    #[tokio::test]
    #[cfg(not(debug_assertions))]
    async fn tri_schedule_in_tier_3() {
        schedule_in_tier::<3>(3).await;
    }

    #[tokio::test]
    async fn bi_schedule_in_tier_0() {
        schedule_in_tier::<2>(0).await;
    }
    #[tokio::test]
    async fn bi_schedule_in_tier_1() {
        schedule_in_tier::<2>(1).await;
    }
    #[tokio::test]
    async fn bi_schedule_in_tier_2() {
        schedule_in_tier::<2>(2).await;
    }

    #[tokio::test]
    async fn uni_schedule_in_tier_0() {
        schedule_in_tier::<1>(0).await;
    }
    #[tokio::test]
    async fn uni_schedule_in_tier_1() {
        schedule_in_tier::<1>(1).await;
    }

    #[tokio::test]
    async fn schedule_tier_0_and_1() {
        let time = Time::new();

        let mut timeq = QuadTimeQ::new(Duration::from_secs(3));
        assert!(timeq.is_empty());

        let intervals = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 768 * 2];
        for &seconds in &intervals {
            let msg = time.new_msg_due_in(Duration::from_secs(seconds)).await;
            time.insert(&mut timeq, msg.clone()).unwrap();
        }

        assert_eq!(timeq.len(), intervals.len());

        let mut popped = vec![];
        loop {
            time.advance(Duration::from_secs(1)).await;
            let mut ready = time.pop(&mut timeq);
            while let Some(_msg) = ready.pop_front() {
                popped.push(time.elapsed());
            }

            if timeq.is_empty() {
                break;
            }
        }

        eprintln!("{popped:?} vs {intervals:?}");

        assert_eq!(popped.len(), intervals.len());

        for (idx, (expected, actual)) in intervals.iter().zip(popped.iter()).enumerate() {
            let upper_limit = Duration::from_secs((*expected as u64).div_ceil(3) * 3);
            assert!(
                *actual >= Duration::from_secs(*expected as u64) && *actual <= upper_limit,
                "idx={idx}, expected {expected}-{upper_limit:?} seconds, got {actual:?}"
            );
        }
    }

    #[tokio::test]
    async fn schedule_cusp() {
        let time = Time::new();

        let msg1 = time.new_msg_due_in(Duration::from_millis(2)).await;
        eprintln!("msg is due: {:?}", msg1.get_due());

        let mut timeq = QuadTimeQ::new(Duration::from_millis(1));

        time.insert(&mut timeq, msg1.clone()).unwrap();
        assert_eq!(timeq.len(), 1);

        assert!(time.pop(&mut timeq).is_empty());

        time.advance(Duration::from_millis(1)).await;
        let ready_list = time.pop(&mut timeq);
        assert_eq!(ready_list.len(), 0);

        time.advance(Duration::from_millis(1)).await;
        let mut ready_list = time.pop(&mut timeq);
        assert_eq!(ready_list.len(), 1);

        let msg = ready_list.pop_front().unwrap();
        let due = msg.get_due().unwrap();
        let now_utc = time.now_utc();

        assert!(
            due <= now_utc,
            "cannot be due in the future. due={due} now={now_utc}"
        );
    }

    #[tokio::test]
    async fn schedule_after_creation() {
        let time = Time::new();

        let mut timeq = QuadTimeQ::new(Duration::from_secs(3));
        assert!(timeq.is_empty());

        let mut popped = vec![];

        // This message will pop at 12 seconds
        let msg = time.new_msg_due_in(Duration::from_secs(10)).await;
        time.insert(&mut timeq, msg.clone()).unwrap();

        time.advance_and_collect(6, &mut timeq, &mut popped).await;

        // This message, although inserted later, will pop at 9 seconds,
        // prior to the message above
        let msg = time.new_msg_due_in(Duration::from_secs(3)).await;
        time.insert(&mut timeq, msg.clone()).unwrap();

        loop {
            time.advance(Duration::from_secs(1)).await;
            let mut ready = time.pop(&mut timeq);
            while let Some(_msg) = ready.pop_front() {
                popped.push(time.elapsed());
            }

            if timeq.is_empty() {
                break;
            }
        }

        let intervals = [9, 12];
        eprintln!("{popped:?} vs {intervals:?}");
        assert_eq!(popped.len(), intervals.len());

        for (expected, actual) in intervals.iter().zip(popped.iter()) {
            let upper_limit = Duration::from_secs((*expected as u64).div_ceil(3) * 3);
            assert!(
                *actual >= Duration::from_secs(*expected as u64) && *actual <= upper_limit,
                "expected {expected}-{upper_limit:?} seconds, got {actual:?}"
            );
        }
    }
}
