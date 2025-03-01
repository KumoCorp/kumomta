use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flume::Sender;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

pub mod local_disk;
#[cfg(feature = "rocksdb")]
pub mod rocks;
pub mod spool_id;

pub use spool_id::SpoolId;

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
