use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flume::Sender;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

pub mod local_disk;
#[cfg(feature = "rocksdb")]
pub mod rocks;
pub mod spool_id;

pub use spool_id::SpoolId;

/// The spool's load-shedding gate is currently latched.  Callers can
/// recognize this via `anyhow::Error::root_cause` (e.g. in the SMTP
/// server) to produce a peer-safe, actionable response instead of a
/// generic internal-error message.  The `Display` impl intentionally
/// matches the bland summary returned by `Spool::unhealthy_reason()`
/// so that the wire-facing text is identical regardless of which
/// layer observes the condition.
#[derive(thiserror::Error, Debug)]
#[error("the spool is not accepting writes")]
pub struct SpoolUnhealthyError;

/// The deadline supplied by the caller (e.g. an SMTP
/// `data_processing_timeout`) elapsed before the spool accepted the
/// write.  Distinguished from [`SpoolBackpressureTimeout`] so that
/// the SMTP server can surface a peer response that accurately
/// blames the caller-side deadline.
#[derive(thiserror::Error, Debug)]
#[error("the caller-provided deadline was reached before the spool accepted the write")]
pub struct SpoolCallerDeadlineExceeded;

/// The spool's own internal backpressure deadline (see
/// `RocksSpoolParams::store_deadline`) elapsed before the write was
/// accepted.  Indicates that the spool itself is the slow component,
/// independently of any caller-supplied deadline; distinguished from
/// [`SpoolCallerDeadlineExceeded`] so the SMTP server can produce a
/// peer response that points the operator at spool health rather
/// than at their own timeout configuration.
#[derive(thiserror::Error, Debug)]
#[error("the spool did not accept the write within {deadline:?}")]
pub struct SpoolBackpressureTimeout {
    pub deadline: Duration,
}

#[derive(Debug)]
pub enum SpoolEntry {
    Item { id: SpoolId, data: Vec<u8> },
    Corrupt { id: SpoolId, error: String },
}

#[async_trait]
pub trait Spool: Send + Sync {
    /// Load the data corresponding to the provided Id
    async fn load(&self, id: SpoolId) -> anyhow::Result<Vec<u8>>;

    /// Remove the data associated with the provided Id
    async fn remove(&self, id: SpoolId) -> anyhow::Result<()>;

    /// Write/Replace the data associated with the provided Id
    async fn store(
        &self,
        id: SpoolId,
        data: Arc<Box<[u8]>>,
        force_sync: bool,
        deadline: Option<Instant>,
    ) -> anyhow::Result<()>;

    /// Scan the contents of the spool, and emit a SpoolEntry for each item
    /// to the provided channel sender.
    /// The items are enumerated in an unspecified order.
    /// It is recommended that you use a bounded channel.
    ///
    /// The results are undefined if you enumerate concurrently with
    /// load/remove/store operations.
    fn enumerate(
        &self,
        sender: Sender<SpoolEntry>,
        start_time: DateTime<Utc>,
    ) -> anyhow::Result<()>;

    /// Perform some periodic cleanup/maintenance
    async fn cleanup(&self) -> anyhow::Result<()>;

    /// Shutdown the store
    async fn shutdown(&self) -> anyhow::Result<()>;

    /// Called when system memory is low.
    /// The spool module should flush and drop caches.
    /// Returns the number of bytes that were saved,
    /// which might be negative if the flush actually
    /// increased the total.
    async fn advise_low_memory(&self) -> anyhow::Result<isize>;

    /// Synchronously flush in-memory buffers and run a full compaction
    /// of the underlying storage.
    ///
    /// Intended for operational diagnostics and for tests that need to
    /// drive the storage into a deterministic state.  Storage backends
    /// that do not have any concept of compaction may leave the default
    /// no-op implementation in place.
    ///
    /// Errors are propagated to the caller.  In particular, for rocksdb,
    /// a missing or corrupt SST file encountered during the operation
    /// surfaces as an `Err`.
    async fn compact(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Returns `None` when the spool is healthy.
    /// Returns `Some(reason)` when the spool is in a state that should
    /// cause ingress paths to shed load.
    ///
    /// This is called from hot load-shedding paths and must be cheap:
    /// no I/O, no awaits, no allocation.  The returned reason is the
    /// externally visible explanation and should be intentionally
    /// bland; it must not leak implementation details.
    fn unhealthy_reason(&self) -> Option<&'static str> {
        None
    }
}

static DATA: OnceLock<Arc<dyn Spool + Send + Sync>> = OnceLock::new();
static META: OnceLock<Arc<dyn Spool + Send + Sync>> = OnceLock::new();

pub fn get_meta_spool() -> &'static Arc<dyn Spool + Send + Sync> {
    META.get().expect("set_meta_spool has not been called")
}

pub fn get_data_spool() -> &'static Arc<dyn Spool + Send + Sync> {
    DATA.get().expect("set_data_spool has not been called")
}

pub fn set_meta_spool(meta: Arc<dyn Spool + Send + Sync>) {
    META.set(meta)
        .map_err(|_| "set_meta_spool has already been called")
        .unwrap();
}

pub fn set_data_spool(data: Arc<dyn Spool + Send + Sync>) {
    DATA.set(data)
        .map_err(|_| "set_data_spool has already been called")
        .unwrap();
}
