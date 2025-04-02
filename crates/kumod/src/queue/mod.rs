use crate::lua_deliver::LuaDeliveryProtocol;
use chrono::{DateTime, Utc};
use message::Message;
use serde::Serialize;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::time::timeout_at;

pub mod config;
pub mod delivery_proto;
pub mod insert_context;
pub mod maintainer;
pub mod manager;
pub mod metrics;
pub mod queue;
pub mod strategy;
pub mod test;

pub use self::config::QueueConfig;
pub use self::delivery_proto::DeliveryProto;
pub use self::insert_context::{InsertContext, InsertReason};
pub use self::manager::QueueManager;
pub use self::queue::Queue;

#[derive(Error, Debug)]
#[error("The Ready Queue is full")]
pub struct ReadyQueueFull;

#[must_use]
enum InsertResult {
    Delayed,
    Ready(Message),
}

/// There can sometimes be a small (eg: 20ms or so) discrepancy
/// between what the time wheel considers to be ready and what
/// the precise due time of the individual messages shows as
/// their due time.
/// That is expected and fine, however: we want to ensure that
/// the actual time is after the due time of this batch of
/// messages so that the logic after THROTTLE_INSERT_READY_SIG
/// doesn't think that the event handler has explicitly delayed
/// the messages and pushes them into the next retry window.
/// This loop accumulates the longest delay from the batch
/// and sleeps until we are past it.
/// An alternative approach to avoiding that confusion might
/// be to call msg.set_due(None), but there is some additional
/// logic in that method that inspects and manipulates scheduling
/// constraints, so it feels slightly better just wait those
/// few milliseconds here than to trigger more work over there.
async fn wait_for_message_batch(batch: &[Message]) {
    if batch.is_empty() {
        return;
    }
    let now = Utc::now();
    let mut delay = Duration::from_secs(0);
    for msg in batch {
        if let Some(due) = msg.get_due() {
            if let Ok(delta) = (due - now).to_std() {
                delay = delay.max(delta);
            }
        }
    }
    tokio::time::sleep(delay).await;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IncrementAttempts {
    No,
    Yes,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueueState {
    pub context: String,
    pub since: DateTime<Utc>,
}

impl QueueState {
    pub fn new<S: Into<String>>(context: S) -> Self {
        Self {
            context: context.into(),
            since: Utc::now(),
        }
    }
}

#[inline]
pub async fn opt_timeout_at<T>(
    deadline: Option<Instant>,
    fut: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    match deadline {
        Some(expires) => timeout_at(expires.into(), fut).await?,
        None => fut.await,
    }
}
