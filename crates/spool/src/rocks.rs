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
                    let id = SpoolId::from_slice(&key)
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

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn rocks_spool() -> anyhow::Result<()> {
        let location = tempfile::tempdir()?;
        let spool = RocksSpool::new(&location.path(), false)?;

        {
            let id1 = SpoolId::new();

            // Can't load an entry that doesn't exist
            assert_eq!(
                format!("{:#}", spool.load(id1).await.unwrap_err()),
                format!("no such key {id1}")
            );
        }

        // Insert some entries
        let mut ids = vec![];
        for i in 0..100 {
            let id = SpoolId::new();
            spool.store(id, format!("I am {i}").as_bytes()).await?;
            ids.push(id);
        }

        // Verify that we can load those entries
        for (i, &id) in ids.iter().enumerate() {
            let data = spool.load(id).await?;
            let text = String::from_utf8(data)?;
            assert_eq!(text, format!("I am {i}"));
        }

        {
            // Verify that we can enumerate them
            let (tx, mut rx) = tokio::sync::mpsc::channel(32);
            spool.enumerate(tx)?;
            let mut count = 0;

            while let Some(item) = rx.recv().await {
                match item {
                    SpoolEntry::Item { id, data } => {
                        let i = ids
                            .iter()
                            .position(|&item| item == id)
                            .ok_or_else(|| anyhow::anyhow!("{id} not found in ids!"))?;

                        let text = String::from_utf8(data)?;
                        assert_eq!(text, format!("I am {i}"));

                        spool.remove(id).await?;
                        // Can't load an entry that we just removed
                        assert_eq!(
                            format!("{:#}", spool.load(id).await.unwrap_err()),
                            format!("no such key {id}")
                        );
                        count += 1;
                    }
                    SpoolEntry::Corrupt { id, error } => {
                        anyhow::bail!("Corrupt: {id}: {error}");
                    }
                }
            }

            assert_eq!(count, 100);
        }

        // Now that we've removed the files, try enumerating again.
        // We expect to receive no entries.
        // Do it a couple of times to verify that none of the cleanup
        // stuff that happens in enumerate breaks the directory
        // structure
        for _ in 0..2 {
            // Verify that we can enumerate them
            let (tx, mut rx) = tokio::sync::mpsc::channel(32);
            spool.enumerate(tx)?;
            let mut unexpected = vec![];

            while let Some(item) = rx.recv().await {
                match item {
                    SpoolEntry::Item { id, .. } | SpoolEntry::Corrupt { id, .. } => {
                        unexpected.push(id)
                    }
                }
            }

            assert_eq!(unexpected.len(), 0);
        }

        Ok(())
    }
}
