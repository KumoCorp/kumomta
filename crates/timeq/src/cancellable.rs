//! Essentially a verbatim copy of hierarchical_hash_wheel_timer::wheels::cancellable.rs,
//! but with Rc replaced with Arc

pub use hierarchical_hash_wheel_timer::wheels::cancellable::CancellableTimerEntry;
use hierarchical_hash_wheel_timer::wheels::quad_wheel::{
    PruneDecision, QuadWheelWithOverflow as BasicQuadWheelWithOverflow,
};
use hierarchical_hash_wheel_timer::wheels::{Skip, TimerEntryWithDelay};
use hierarchical_hash_wheel_timer::TimerError;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Weak};
use std::time::Duration;

/// A pruner implementation for [Weak](std::sync::Weak) references
///
/// Keeps values that can still be upgraded.
fn arc_prune<E>(e: &Weak<E>) -> PruneDecision {
    if e.strong_count() > 0 {
        PruneDecision::Keep
    } else {
        PruneDecision::Drop
    }
}

/// An implementation of four-level byte-sized wheel
///
/// Any value scheduled so far off that it doesn't fit into the wheel
/// is stored in an overflow `Vec` and added to the wheel, once time as advanced enough
/// that it actually fits.
/// In this design the maximum schedule duration for the wheel itself is [`u32::MAX`](std::u32::MAX) units (typically ms),
/// everything else goes into the overflow `Vec`.
pub struct QuadWheelWithOverflow<EntryType>
where
    EntryType: CancellableTimerEntry,
{
    wheel: BasicQuadWheelWithOverflow<Weak<EntryType>>,
    timers: HashMap<EntryType::Id, Arc<EntryType>>,
}

impl<EntryType> QuadWheelWithOverflow<EntryType>
where
    EntryType: TimerEntryWithDelay + CancellableTimerEntry,
{
    /// Insert a new timeout into the wheel
    pub fn insert(&mut self, e: EntryType) -> Result<(), TimerError<EntryType>> {
        self.insert_ref(Arc::new(e)).map_err(|err| match err {
            TimerError::Expired(rc_e) => {
                let e =
                    Arc::try_unwrap(rc_e).expect("safe because we're the only one to have a ref");
                TimerError::Expired(e)
            }
            TimerError::NotFound => TimerError::NotFound,
        })
    }

    /// Insert a new timeout into the wheel
    pub fn insert_ref(&mut self, e: Arc<EntryType>) -> Result<(), TimerError<Arc<EntryType>>> {
        let delay = e.delay();
        self.insert_ref_with_delay(e, delay)
    }
}

impl<EntryType> QuadWheelWithOverflow<EntryType>
where
    EntryType: CancellableTimerEntry,
{
    /// Create a new wheel
    pub fn new() -> Self {
        QuadWheelWithOverflow {
            wheel: BasicQuadWheelWithOverflow::new(arc_prune::<EntryType>),
            timers: HashMap::new(),
        }
    }

    /// Insert a new timeout into the wheel to be returned after `delay` ticks
    pub fn insert_ref_with_delay(
        &mut self,
        e: Arc<EntryType>,
        delay: Duration,
    ) -> Result<(), TimerError<Arc<EntryType>>> {
        let weak_e = Arc::downgrade(&e);

        match self.wheel.insert_with_delay(weak_e, delay) {
            Ok(_) => {
                self.timers.insert(e.id().clone(), e);
                Ok(())
            }
            Err(TimerError::Expired(_weak_e)) => Err(TimerError::Expired(e)),
            // not that this can happen here, but it makes the compiler happy
            Err(TimerError::NotFound) => Err(TimerError::NotFound),
        }
    }

    /// Cancel the timeout with the given `id`
    ///
    /// This method is very cheap, as it doesn't actually touch the wheels at all.
    /// It simply removes the value from the lookup table, so it can't be executed
    /// once its triggered. This also automatically prevents rescheduling of periodic timeouts.
    pub fn cancel(&mut self, id: &EntryType::Id) -> Result<(), TimerError<Infallible>> {
        // Simply remove it from the lookup table
        // This will prevent the Weak pointer in the wheels from upgrading later
        match self.timers.remove_entry(id) {
            Some(_) => Ok(()),
            None => Err(TimerError::NotFound),
        }
    }

    fn take_timer(&mut self, weak_e: Weak<EntryType>) -> Option<Arc<EntryType>> {
        match weak_e.upgrade() {
            Some(rc_e) => {
                match self.timers.remove_entry(rc_e.id()) {
                    Some(rc_e2) => drop(rc_e2), // ok
                    None => {
                        // Perhaps it was removed via cancel(), and the underlying
                        // Arc is still alive through some other reference
                        return None;
                    }
                }
                Some(rc_e)
            }
            None => None,
        }
    }

    /// Move the wheel forward by a single unit (ms)
    ///
    /// Returns a list of all timers that expire during this tick.
    pub fn tick(&mut self) -> Vec<Arc<EntryType>> {
        let res = self.wheel.tick();
        res.into_iter()
            .flat_map(|weak_e| self.take_timer(weak_e))
            .collect()
    }

    /// Skip a certain `amount` of units (ms)
    ///
    /// No timers will be executed for the skipped time.
    /// Only use this after determining that it's actually
    /// valid with [can_skip](QuadWheelWithOverflow::can_skip)!
    pub fn skip(&mut self, amount: u32) -> () {
        self.wheel.skip(amount);
    }

    /// Determine if and how many ticks can be skipped
    pub fn can_skip(&self) -> Skip {
        self.wheel.can_skip()
    }
}
