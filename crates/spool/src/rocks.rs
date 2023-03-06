use crate::{Spool, SpoolEntry, SpoolId};
use async_trait::async_trait;
use rocksdb::{IteratorMode, Options, DB};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub struct RocksSpool {
    db: Arc<DB>,
}

impl RocksSpool {
    pub fn new(path: &Path, flush: bool) -> anyhow::Result<Self> {
        let mut opts = Options::default();
        opts.set_use_fsync(flush);
        opts.create_if_missing(true);
        let db = Arc::new(DB::open(&opts, path)?);

        Ok(Self { db })
    }
}

#[async_trait]
impl Spool for RocksSpool {
    async fn load(&self, id: SpoolId) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .db
            .get(id.as_bytes())?
            .ok_or_else(|| anyhow::anyhow!("no such key {id}"))?)
    }

    async fn store(&self, id: SpoolId, data: &[u8]) -> anyhow::Result<()> {
        self.db.put(id.as_bytes(), data)?;
        Ok(())
    }

    async fn remove(&self, id: SpoolId) -> anyhow::Result<()> {
        self.db.delete(id.as_bytes())?;
        Ok(())
    }

    async fn cleanup(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn enumerate(&self, sender: Sender<SpoolEntry>) -> anyhow::Result<()> {
        let db = Arc::clone(&self.db);
        tokio::task::Builder::new()
            .name("rocksdb enumerate")
            .spawn_blocking(move || {
                let iter = db.iterator(IteratorMode::Start);
                for entry in iter {
                    let (key, value) = entry?;
                    let id = SpoolId::from_ascii_bytes(&key)
                        .ok_or_else(|| anyhow::anyhow!("invalid spool id {key:?}"))?;
                    sender
                        .blocking_send(SpoolEntry::Item {
                            id,
                            data: value.to_vec(),
                        })
                        .map_err(|err| {
                            anyhow::anyhow!("failed to send SpoolEntry for {id}: {err:#}")
                        })?;
                }
                Ok::<(), anyhow::Error>(())
            })?;
        Ok(())
    }
}
