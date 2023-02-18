use hierarchical_hash_wheel_timer::wheels::Skip;
pub use hierarchical_hash_wheel_timer::TimerError;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use hierarchical_hash_wheel_timer::wheels::cancellable::CancellableTimerEntry;
pub use hierarchical_hash_wheel_timer::wheels::TimerEntryWithDelay;

mod cancellable;

pub use crate::cancellable::QuadWheelWithOverflow;

/// A TimeQ is a queue datastructure where the contained items are time
/// ordered.
/// The underlying storage is a hashed hierarchical timer wheel, which
/// allows for relatively cheap insertion and popping of ready items.
/// It is also possible to cancel an entry given its id.
pub struct TimeQ<EntryType: CancellableTimerEntry + TimerEntryWithDelay> {
    wheel: QuadWheelWithOverflow<EntryType>,
    start: Instant,
    last_check: u128,
    len: usize,
}

#[must_use]
pub enum PopResult<EntryType> {
    /// These items are ready for immediate action
    Items(Vec<Arc<EntryType>>),
    /// No items will be ready for the specified duration
    Sleep(Duration),
    /// The queue is empty
    Empty,
}

impl<EntryType: CancellableTimerEntry + TimerEntryWithDelay> TimeQ<EntryType> {
    pub fn new() -> Self {
        Self {
            wheel: QuadWheelWithOverflow::new(),
            start: Instant::now(),
            last_check: 0,
            len: 0,
        }
    }

    fn elapsed(&mut self) -> u128 {
        let since_start = self.start.elapsed().as_millis();
        let relative = since_start - self.last_check;
        self.last_check = since_start;
        relative
    }

    /// Returns true if the wheel is empty
    pub fn is_empty(&self) -> bool {
        matches!(self.wheel.can_skip(), Skip::Empty)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Insert a new entry
    pub fn insert(&mut self, entry: Arc<EntryType>) -> Result<(), TimerError<Arc<EntryType>>> {
        self.wheel.insert_ref(entry)?;
        self.len += 1;
        Ok(())
    }

    /// Cancel an item given its id
    pub fn cancel_by_id(
        &mut self,
        id: &EntryType::Id,
    ) -> Result<(), TimerError<std::convert::Infallible>> {
        self.wheel.cancel(id)?;
        self.len -= 1;
        Ok(())
    }

    /// Returns the set of items that need immediate action
    pub fn pop(&mut self) -> PopResult<EntryType> {
        let elapsed = self.elapsed();
        if elapsed > 0 {
            let mut items = vec![];

            let mut elapsed = elapsed as u32;
            while elapsed > 0 {
                match self.wheel.can_skip() {
                    Skip::Empty => break,
                    Skip::None => {
                        items.append(&mut self.wheel.tick());
                        elapsed -= 1;
                    }
                    Skip::Millis(m) => {
                        let amount = m.min(elapsed);
                        self.wheel.skip(amount);
                        elapsed -= amount;
                    }
                }
            }

            if !items.is_empty() {
                self.len -= items.len();
                return PopResult::Items(items);
            }
        }

        match self.wheel.can_skip() {
            Skip::None => PopResult::Sleep(Duration::from_millis(1)),
            Skip::Empty => PopResult::Empty,
            Skip::Millis(ms) => PopResult::Sleep(Duration::from_millis(ms.into())),
        }
    }

    /// Drains the entire contents of the queue, returning all of the
    /// contained items
    pub fn drain(&mut self) -> Vec<Arc<EntryType>> {
        let mut items = vec![];
        loop {
            match self.wheel.can_skip() {
                Skip::Empty => {
                    self.start = Instant::now();
                    self.last_check = 0;
                    self.len = 0;
                    break;
                }
                Skip::None => {
                    items.append(&mut self.wheel.tick());
                }
                Skip::Millis(m) => {
                    self.wheel.skip(m);
                }
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Entry {
        id: u64,
        value: &'static str,
        delay: Duration,
    }

    impl CancellableTimerEntry for Entry {
        type Id = u64;
        fn id(&self) -> &Self::Id {
            &self.id
        }
    }

    impl TimerEntryWithDelay for Entry {
        fn delay(&self) -> Duration {
            self.delay
        }
    }

    #[test]
    fn draining() {
        let item1 = Arc::new(Entry {
            id: 1,
            value: "foo",
            delay: Duration::from_millis(1),
        });
        let item2 = Arc::new(Entry {
            id: 2,
            value: "bar",
            delay: Duration::from_millis(10),
        });
        let item3 = Arc::new(Entry {
            id: 3,
            value: "baz",
            delay: Duration::from_millis(5),
        });

        let mut queue = TimeQ::new();
        queue.insert(Arc::clone(&item1)).unwrap();
        queue.insert(Arc::clone(&item2)).unwrap();
        queue.insert(Arc::clone(&item3)).unwrap();

        let items = queue.drain();
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.is_empty(), true);
        assert_eq!(items, vec![item1, item3, item2]);
    }

    #[test]
    fn cancel() {
        let item1 = Arc::new(Entry {
            id: 1,
            value: "foo",
            delay: Duration::from_millis(1),
        });
        let item2 = Arc::new(Entry {
            id: 2,
            value: "bar",
            delay: Duration::from_millis(10),
        });
        let item3 = Arc::new(Entry {
            id: 3,
            value: "baz",
            delay: Duration::from_millis(5),
        });

        let mut queue = TimeQ::new();
        queue.insert(Arc::clone(&item1)).unwrap();
        queue.insert(Arc::clone(&item2)).unwrap();
        queue.insert(Arc::clone(&item3)).unwrap();
        queue.cancel_by_id(&item2.id).unwrap();

        let items = queue.drain();
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.is_empty(), true);
        assert_eq!(items, vec![item1, item3]);
    }

    #[test]
    fn basic_queue() {
        let mut queue = TimeQ::new();

        let item1 = Arc::new(Entry {
            id: 1,
            value: "foo",
            delay: Duration::from_millis(1),
        });
        let item2 = Arc::new(Entry {
            id: 2,
            value: "bar",
            delay: Duration::from_millis(10),
        });
        let item3 = Arc::new(Entry {
            id: 3,
            value: "baz",
            delay: Duration::from_millis(5),
        });

        queue.insert(Arc::clone(&item1)).unwrap();
        queue.insert(Arc::clone(&item2)).unwrap();
        queue.insert(Arc::clone(&item3)).unwrap();

        assert_eq!(queue.len(), 3);
        assert_eq!(queue.is_empty(), false);

        std::thread::sleep(Duration::from_millis(2));

        match queue.pop() {
            PopResult::Items(items) => assert_eq!(items, vec![item1]),
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
        std::thread::sleep(Duration::from_millis(2));

        match queue.pop() {
            PopResult::Items(items) => assert_eq!(items, vec![item3]),
            PopResult::Sleep(ms) => assert!(false, "still have {ms:?} to go"),
            _ => unreachable!(),
        }

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.is_empty(), false);

        std::thread::sleep(Duration::from_millis(10));
        match queue.pop() {
            PopResult::Items(items) => assert_eq!(items, vec![item2]),
            _ => unreachable!(),
        }

        assert_eq!(queue.len(), 0);
        assert_eq!(queue.is_empty(), true);
    }
}
