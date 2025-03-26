use hierarchical_hash_wheel_timer::wheels::quad_wheel::no_prune;
use hierarchical_hash_wheel_timer::wheels::Skip;
pub use hierarchical_hash_wheel_timer::TimerError;
use std::time::{Duration, Instant};

pub use hierarchical_hash_wheel_timer::wheels::quad_wheel::QuadWheelWithOverflow;
pub use hierarchical_hash_wheel_timer::wheels::TimerEntryWithDelay;

/// A TimeQ is a queue datastructure where the contained items are time
/// ordered.
/// The underlying storage is a hashed hierarchical timer wheel, which
/// allows for relatively cheap insertion and popping of ready items.
/// It is also possible to cancel an entry given its id.
pub struct TimeQ<EntryType: TimerEntryWithDelay> {
    wheel: QuadWheelWithOverflow<EntryType>,
    last_check: Instant,
    len: usize,
}

#[must_use]
pub enum PopResult<EntryType> {
    /// These items are ready for immediate action
    Items(Vec<EntryType>),
    /// No items will be ready for the specified duration
    Sleep(Duration),
    /// The queue is empty
    Empty,
}

impl<EntryType: TimerEntryWithDelay> TimeQ<EntryType> {
    pub fn new() -> Self {
        Self {
            wheel: QuadWheelWithOverflow::new(no_prune),
            last_check: Instant::now(),
            len: 0,
        }
    }

    /// Returns true if the wheel is empty
    pub fn is_empty(&self) -> bool {
        matches!(self.wheel.can_skip(), Skip::Empty)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Insert a new entry
    pub fn insert(&mut self, entry: EntryType) -> Result<(), TimerError<EntryType>> {
        self.wheel.insert(entry)?;
        self.len += 1;
        Ok(())
    }

    /// Returns the set of items that need immediate action
    pub fn pop(&mut self) -> PopResult<EntryType> {
        let now = Instant::now();
        let elapsed = now - self.last_check;
        self.last_check = now;
        let mut elapsed_ms = elapsed.as_millis() as u32;

        let mut items = vec![];

        while elapsed_ms > 0 {
            match self.wheel.can_skip() {
                Skip::Empty => break,
                Skip::None => {
                    items.append(&mut self.wheel.tick());
                    elapsed_ms -= 1;
                }
                Skip::Millis(m) => {
                    let amount = m.min(elapsed_ms);
                    self.wheel.skip(amount);
                    elapsed_ms -= amount;
                }
            }
        }

        if !items.is_empty() {
            self.len -= items.len();
            return PopResult::Items(items);
        }

        match self.wheel.can_skip() {
            Skip::None => PopResult::Sleep(Duration::from_millis(1)),
            Skip::Empty => PopResult::Empty,
            Skip::Millis(ms) => PopResult::Sleep(Duration::from_millis(ms.into())),
        }
    }

    /// Drains the entire contents of the queue, returning all of the
    /// contained items
    pub fn drain(&mut self) -> Vec<EntryType> {
        let mut items = vec![];
        loop {
            match self.wheel.can_skip() {
                Skip::Empty => {
                    self.last_check = Instant::now();
                    self.len = 0;
                    return items;
                }
                Skip::None => {
                    items.append(&mut self.wheel.tick());
                }
                Skip::Millis(m) => {
                    self.wheel.skip(m);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Clone)]
    struct Entry {
        id: u64,
        value: &'static str,
        delay: Duration,
    }

    impl TimerEntryWithDelay for &Entry {
        fn delay(&self) -> Duration {
            self.delay
        }
    }

    #[test]
    fn draining() {
        let item1 = Entry {
            id: 1,
            value: "foo",
            delay: Duration::from_millis(1),
        };
        let item2 = Entry {
            id: 2,
            value: "bar",
            delay: Duration::from_millis(10),
        };
        let item3 = Entry {
            id: 3,
            value: "baz",
            delay: Duration::from_millis(5),
        };

        let mut queue = TimeQ::new();
        queue.insert(&item1).unwrap();
        queue.insert(&item2).unwrap();
        queue.insert(&item3).unwrap();

        let items = queue.drain();
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.is_empty(), true);
        assert_eq!(items, vec![&item1, &item3, &item2]);
    }

    #[test]
    fn basic_queue() {
        let mut queue = TimeQ::new();

        let item1 = Entry {
            id: 1,
            value: "foo",
            delay: Duration::from_millis(1),
        };
        let item2 = Entry {
            id: 2,
            value: "bar",
            delay: Duration::from_secs(1),
        };
        let item3 = Entry {
            id: 3,
            value: "baz",
            delay: Duration::from_millis(100),
        };

        queue.insert(&item1).unwrap();
        queue.insert(&item2).unwrap();
        queue.insert(&item3).unwrap();

        assert_eq!(queue.len(), 3);
        assert_eq!(queue.is_empty(), false);

        std::thread::sleep(Duration::from_millis(2));

        match queue.pop() {
            PopResult::Items(items) => assert_eq!(items, vec![&item1]),
            _ => unreachable!(),
        }

        assert_eq!(queue.len(), 2);
        assert_eq!(queue.is_empty(), false);

        std::thread::sleep(Duration::from_millis(2));

        match queue.pop() {
            PopResult::Sleep(ms) => std::thread::sleep(ms),
            _ => unreachable!(),
        }

        // The PopResult::Sleep is approximate and often doesn't
        // quite get us there, so sleep slightly longer
        std::thread::sleep(Duration::from_millis(100));

        match queue.pop() {
            PopResult::Items(items) => assert_eq!(items, vec![&item3]),
            PopResult::Sleep(ms) => assert!(false, "still have {ms:?} to go"),
            _ => unreachable!(),
        }

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.is_empty(), false);

        std::thread::sleep(Duration::from_secs(1));
        match queue.pop() {
            PopResult::Items(items) => assert_eq!(items, vec![&item2]),
            _ => unreachable!(),
        }

        assert_eq!(queue.len(), 0);
        assert_eq!(queue.is_empty(), true);
    }
}
