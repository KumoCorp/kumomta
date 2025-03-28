use crate::queue::maintainer::{start_singleton_wheel_v1, start_singleton_wheel_v2};
use chrono::{DateTime, Utc};
use crossbeam_skiplist::SkipSet;
use message::message::WeakMessage;
use message::timeq::TriTimeQ;
use message::Message;
use mlua::prelude::*;
use parking_lot::FairMutex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use timeq::{PopResult, TimeQ, TimerError};

pub static SINGLETON_WHEEL: LazyLock<Arc<FairMutex<TimeQ<WeakMessage>>>> =
    LazyLock::new(|| Arc::new(FairMutex::new(TimeQ::new())));

pub static SINGLETON_WHEEL_V2: LazyLock<Arc<FairMutex<TriTimeQ>>> =
    LazyLock::new(|| Arc::new(FairMutex::new(TriTimeQ::new(Duration::from_secs(3)))));
const ZERO_DURATION: Duration = Duration::from_secs(0);

#[derive(Deserialize, Serialize, Debug, Clone, FromLua, Default, Copy, PartialEq, Eq)]
pub enum QueueStrategy {
    TimerWheel,
    SkipList,
    #[default]
    SingletonTimerWheel,
    SingletonTimerWheelV2,
}

#[must_use]
#[derive(Debug)]
pub enum QueueInsertResult {
    Inserted { should_notify: bool },
    Full(Message),
}

pub enum QueueStructure {
    TimerWheel(FairMutex<TimeQ<Message>>),
    SkipList(SkipSet<DelayedEntry>),
    SingletonTimerWheel(FairMutex<HashSet<Message>>),
    SingletonTimerWheelV2(FairMutex<HashSet<Message>>),
}

impl QueueStructure {
    pub fn new(strategy: QueueStrategy) -> Self {
        match strategy {
            QueueStrategy::TimerWheel => Self::TimerWheel(FairMutex::new(TimeQ::new())),
            QueueStrategy::SkipList => Self::SkipList(SkipSet::new()),
            QueueStrategy::SingletonTimerWheel => {
                Self::SingletonTimerWheel(FairMutex::new(HashSet::new()))
            }
            QueueStrategy::SingletonTimerWheelV2 => {
                Self::SingletonTimerWheelV2(FairMutex::new(HashSet::new()))
            }
        }
    }

    pub fn shrink(&self) {
        match self {
            Self::TimerWheel(_q) => {}
            Self::SkipList(_q) => {}
            Self::SingletonTimerWheel(q) => {
                q.lock().shrink_to_fit();
            }
            Self::SingletonTimerWheelV2(q) => {
                q.lock().shrink_to_fit();
            }
        }
    }

    pub fn pop(&self) -> (Vec<Message>, Option<Duration>) {
        match self {
            Self::TimerWheel(q) => match q.lock().pop() {
                PopResult::Items(messages) => (messages, None),
                PopResult::Sleep(_) | PopResult::Empty => (vec![], None),
            },
            Self::SkipList(q) => {
                let now = Utc::now();
                let mut messages = vec![];
                let mut sleep_duration = None;

                while let Some(entry) = q.front() {
                    let delay = entry.compute_delay(now);
                    if delay == ZERO_DURATION {
                        entry.remove();
                        messages.push(entry.0.clone());
                    } else {
                        sleep_duration = Some(delay);
                        break;
                    }
                }

                (messages, sleep_duration)
            }
            Self::SingletonTimerWheel(_) => (vec![], None),
            Self::SingletonTimerWheelV2(_) => (vec![], None),
        }
    }

    pub fn drain(&self) -> Vec<Message> {
        match self {
            Self::TimerWheel(q) => q.lock().drain(),
            Self::SkipList(q) => {
                let mut msgs = vec![];
                while let Some(entry) = q.pop_front() {
                    msgs.push((*entry).0.clone());
                }
                msgs
            }
            Self::SingletonTimerWheel(q) => q.lock().drain().collect(),
            Self::SingletonTimerWheelV2(q) => {
                // Note: We must always lock SINGLETON_WHEEL_2 before q
                let mut wheel = SINGLETON_WHEEL_V2.lock();
                let mut q = q.lock();
                let mut msgs: Vec<Message> = q.drain().collect();
                // run_singleton_wheel_v2 does not guarantee that it
                // will atomically remove a msg from the wheel and q.
                // It removes from the wheel first, then subsequently
                // resolves q to fix it up for ready messages.
                // If the wheel.cancel call fails, the message we are
                // considering is in-flight over in the run_singleton_wheel_v2
                // and we must not include it here, and need to put it
                // back into q so that things can process correctly.
                msgs.retain(|msg| {
                    if wheel.cancel(&msg) {
                        true
                    } else {
                        // Put it back in the queue so that
                        // run_singleton_wheel_v2 can find it
                        // and process it.
                        q.insert(msg.clone());
                        false
                    }
                });
                msgs
            }
        }
    }

    pub fn iter(&self, take: Option<usize>) -> Vec<Message> {
        match self {
            Self::TimerWheel(_) => vec![],
            Self::SkipList(_) => vec![],
            Self::SingletonTimerWheel(q) => q
                .lock()
                .iter()
                .take(take.unwrap_or(usize::MAX))
                .cloned()
                .collect(),
            Self::SingletonTimerWheelV2(q) => {
                let wheel = SINGLETON_WHEEL_V2.lock();
                q.lock()
                    .iter()
                    .take(take.unwrap_or(usize::MAX))
                    .filter_map(|msg| {
                        if wheel.contains(msg) {
                            Some(msg.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            }
        }
    }

    pub fn insert(&self, msg: Message) -> QueueInsertResult {
        match self {
            Self::TimerWheel(q) => match q.lock().insert(msg) {
                Ok(()) => QueueInsertResult::Inserted {
                    // We never notify for TimerWheel because we always tick
                    // on a regular(ish) schedule
                    should_notify: false,
                },
                Err(TimerError::Expired(msg)) => QueueInsertResult::Full(msg),
                Err(TimerError::NotFound) => unreachable!(),
            },
            Self::SkipList(q) => {
                let now_ts = Utc::now().timestamp();
                let entry = DelayedEntry(msg);
                let entry_due = entry.get_bucketed_due();
                if entry_due <= now_ts {
                    QueueInsertResult::Full(entry.0)
                } else {
                    let due = q.front().map(|entry| entry.get_bucketed_due());
                    q.insert(entry);
                    let now_due = q.front().map(|entry| entry.get_bucketed_due());
                    QueueInsertResult::Inserted {
                        // Only notify the maintainer if it now needs to wake up
                        // sooner than it previously thought. In particular,
                        // we do not want to wake up for every message insertion,
                        // as that would generally be a waste of effort and bog
                        // down the system without gain.
                        should_notify: if due.is_none() { true } else { now_due < due },
                    }
                }
            }
            Self::SingletonTimerWheel(q) => {
                // Ensure that the msg is visible in q before we add it to
                // the timer wheel, as it is possible for it to tick and pop
                // the message as soon as it is inserted into the wheel.
                q.lock().insert(msg.clone());
                match SINGLETON_WHEEL.lock().insert(msg.weak()) {
                    Ok(()) => {
                        start_singleton_wheel_v1();
                        QueueInsertResult::Inserted {
                            // We never notify for TimerWheel because we always tick
                            // on a regular(ish) schedule
                            should_notify: false,
                        }
                    }
                    Err(TimerError::Expired(_weak_msg)) => {
                        // Message is actually due immediately.
                        // Take it out of the local q and return it
                        q.lock().remove(&msg);
                        QueueInsertResult::Full(msg)
                    }
                    Err(TimerError::NotFound) => unreachable!(),
                }
            }
            Self::SingletonTimerWheelV2(q) => {
                let mut wheel = SINGLETON_WHEEL_V2.lock();
                match wheel.insert(msg.clone()) {
                    Ok(()) => {
                        q.lock().insert(msg);
                        drop(wheel);
                        start_singleton_wheel_v2();

                        QueueInsertResult::Inserted {
                            // We never notify for TimerWheel because we always tick
                            // on a regular(ish) schedule
                            should_notify: false,
                        }
                    }
                    Err(msg) => {
                        // Message is actually due immediately.
                        QueueInsertResult::Full(msg)
                    }
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::TimerWheel(q) => q.lock().len(),
            Self::SkipList(q) => q.len(),
            Self::SingletonTimerWheel(q) => q.lock().len(),
            Self::SingletonTimerWheelV2(q) => q.lock().len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::TimerWheel(q) => q.lock().is_empty(),
            Self::SkipList(q) => q.is_empty(),
            Self::SingletonTimerWheel(q) => q.lock().is_empty(),
            Self::SingletonTimerWheelV2(q) => q.lock().is_empty(),
        }
    }

    pub fn is_timer_wheel(&self) -> bool {
        matches!(self, Self::TimerWheel(_))
    }

    pub fn strategy(&self) -> QueueStrategy {
        match self {
            Self::TimerWheel(_) => QueueStrategy::TimerWheel,
            Self::SkipList(_) => QueueStrategy::SkipList,
            Self::SingletonTimerWheel(_) => QueueStrategy::SingletonTimerWheel,
            Self::SingletonTimerWheelV2(_) => QueueStrategy::SingletonTimerWheelV2,
        }
    }
}

#[derive(Debug)]
pub struct DelayedEntry(Message);

impl DelayedEntry {
    /// Get the due time with lower granularity than the underlying
    /// timestamp allows.
    /// Here it is 1 second.  For sites with very large
    /// scheduled queues and reasonable retry intervals
    /// it is desirable to reduce the granularity beacuse
    /// it makes the cost of the skiplist insertion
    /// cheaper when multiple items compare equal: we can insert
    /// when we find the start of a batch with the same second
    fn get_bucketed_due(&self) -> i64 {
        self.0.get_due().map(|d| d.timestamp()).unwrap_or(0)
    }

    fn compute_delay(&self, now: DateTime<Utc>) -> Duration {
        let due = self.get_bucketed_due();
        let now_ts = now.timestamp();
        Duration::from_secs(due.saturating_sub(now_ts).max(0) as u64)
    }
}

impl PartialEq for DelayedEntry {
    fn eq(&self, other: &DelayedEntry) -> bool {
        self.get_bucketed_due().eq(&other.get_bucketed_due())
    }
}
impl Eq for DelayedEntry {}
impl PartialOrd for DelayedEntry {
    fn partial_cmp(&self, other: &DelayedEntry) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DelayedEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get_bucketed_due().cmp(&other.get_bucketed_due())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use message::EnvelopeAddress;
    use spool::SpoolId;

    async fn insert_past_due(qs: &mut QueueStructure) {
        let msg = Message::new_dirty(
            SpoolId::new(),
            EnvelopeAddress::parse("sender@example.com").unwrap(),
            EnvelopeAddress::parse("recip@example.com").unwrap(),
            serde_json::json!({}),
            Arc::new(
                "Subject: hello\r\n\r\nwoot"
                    .as_bytes()
                    .to_vec()
                    .into_boxed_slice(),
            ),
        )
        .unwrap();

        let due = msg
            .delay_by(kumo_chrono_helper::seconds(-30).unwrap())
            .await
            .unwrap();
        eprintln!("due {due:?}");
        let result = qs.insert(msg);
        eprintln!("result: {result:?}");
        assert!(matches!(result, QueueInsertResult::Full(_)));
    }

    #[tokio::test]
    async fn insert_past_due_timer_wheel() {
        let mut qs = QueueStructure::new(QueueStrategy::TimerWheel);
        insert_past_due(&mut qs).await;
    }

    #[tokio::test]
    async fn insert_past_due_skip_list() {
        let mut qs = QueueStructure::new(QueueStrategy::SkipList);
        insert_past_due(&mut qs).await;
    }

    // Note: this test vivifies SINGLETON_WHEEL and may have other consequences
    // if other tests do the same. In this case, assuming that things are working
    // correctly, this test doesn't actually mutate it because the message is
    // immediately due.
    #[tokio::test]
    async fn insert_past_due_singleton_timer_wheel_v1() {
        let mut qs = QueueStructure::new(QueueStrategy::SingletonTimerWheel);
        insert_past_due(&mut qs).await;
    }

    // Note: this test vivifies SINGLETON_WHEEL_V2 and may have other consequences
    // if other tests do the same. In this case, assuming that things are working
    // correctly, this test doesn't actually mutate it because the message is
    // immediately due.
    #[tokio::test]
    async fn insert_past_due_singleton_timer_wheel_v2() {
        let mut qs = QueueStructure::new(QueueStrategy::SingletonTimerWheelV2);
        insert_past_due(&mut qs).await;
    }
}
