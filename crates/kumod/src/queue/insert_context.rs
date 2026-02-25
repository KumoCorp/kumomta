use serde::{Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct InsertContext(SmallVec<[InsertReason; 4]>);

impl std::fmt::Display for InsertContext {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (idx, reason) in self.0.iter().enumerate() {
            if idx > 0 {
                write!(fmt, ", ")?;
            }
            write!(fmt, "{reason:?}")?;
        }
        Ok(())
    }
}

impl InsertContext {
    pub fn add(mut self, reason: InsertReason) -> Self {
        self.note(reason);
        self
    }

    pub fn note(&mut self, reason: InsertReason) {
        if self.0.last().copied() != Some(reason) {
            self.0.push(reason);
        }
    }

    pub fn contains(&self, reason: InsertReason) -> bool {
        self.0.contains(&reason)
    }

    pub fn only(&self, reason: InsertReason) -> bool {
        self.contains(reason) && self.0.len() == 1
    }
}

impl From<InsertReason> for InsertContext {
    fn from(reason: InsertReason) -> InsertContext {
        InsertContext(smallvec![reason])
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum InsertReason {
    /// Message was just received
    Received,
    /// Message was just loaded from spool
    Enumerated,
    /// Message had its due time explicitly set.
    /// This reason is synthesized from context
    ScheduledForLater,
    ReadyQueueWasSuspended,
    MessageRateThrottle,
    ThrottledByThrottleInsertReadyQueue,
    ReadyQueueWasFull,
    FailedToInsertIntoReadyQueue,
    MessageGetQueueNameFailed,
    AdminRebind,
    DueTimeWasReached,
    MaxReadyWasReducedByConfigUpdate,
    ReadyQueueWasDelayedDueToLowMemory,
    FailedDueToNullMx,
    MxResolvedToZeroHosts,
    MxWasProhibited,
    MxWasSkipped,
    TooManyConnectionFailures,
    TooManyRecipients,
    ConnectionRateThrottle,
    /// There was a TransientFailure logged to explain what really happened
    LoggedTransientFailure,
    /// Should be impossible to see in practice, because we can only
    /// reap when the queue has no messages in it
    ReadyQueueWasReaped,
    /// The safey net in Dispatcher::Drop re-queued the message.
    /// This shouldn't happen; if you see this in a log, please report it!
    DispatcherDrop,
    /// The peer unilaterally closed the connection before we started
    /// delivery, so we want to try a new connection plan
    PeerClosedConnection,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn serialization() {
        let context: InsertContext = InsertReason::ThrottledByThrottleInsertReadyQueue.into();
        let s = serde_json::to_string(&context).unwrap();
        k9::assert_equal!(s, "[\"ThrottledByThrottleInsertReadyQueue\"]");

        let round_trip: InsertContext = serde_json::from_str(&s).unwrap();
        k9::assert_equal!(context, round_trip);
    }
}
