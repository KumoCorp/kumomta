use crate::{Spool, SpoolEntry, SpoolId};
use anyhow::Context;
use async_trait::async_trait;
use sled::{Config, Db, Mode};
use std::path::Path;
use tokio::sync::mpsc::Sender;

pub struct SledDiskSpool {
    db: Db,
    flush: bool,
}

impl SledDiskSpool {
    pub fn new(path: &Path, flush: bool) -> anyhow::Result<Self> {
        let db = Config::new()
            .path(path)
            .mode(Mode::HighThroughput)
            .use_compression(false)
            .open()
            .with_context(|| format!("opening sled db at {}", path.display()))?;

        Ok(Self { db, flush })
    }
}

#[async_trait]
impl Spool for SledDiskSpool {
    async fn load(&self, id: SpoolId) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .db
            .get(id.to_string())?
            .ok_or_else(|| anyhow::anyhow!("no such key {id}"))?
            .to_vec())
    }

    async fn store(&self, id: SpoolId, data: &[u8]) -> anyhow::Result<()> {
        self.db.insert(id.to_string(), data)?;
        if self.flush {
            self.db.flush_async().await?;
        }
        Ok(())
    }

    async fn remove(&self, id: SpoolId) -> anyhow::Result<()> {
        self.db.remove(id.to_string())?;
        Ok(())
    }

    async fn cleanup(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn enumerate(&self, sender: Sender<SpoolEntry>) -> anyhow::Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            for entry in db.iter() {
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
        });
        Ok(())
    }
}
