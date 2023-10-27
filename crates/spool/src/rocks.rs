use crate::{Spool, SpoolEntry, SpoolId};
use async_trait::async_trait;
use flume::Sender;
use rocksdb::{DBCompressionType, IteratorMode, LogLevel, Options, DB};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug)]
pub struct RocksSpoolParams {
    pub increase_parallelism: Option<i32>,

    pub optimize_level_style_compaction: Option<usize>,
    pub optimize_universal_style_compaction: Option<usize>,
    #[serde(default)]
    pub paranoid_checks: bool,
    #[serde(default)]
    pub compression_type: DBCompressionTypeDef,

    /// If non-zero, we perform bigger reads when doing compaction. If you’re running RocksDB on
    /// spinning disks, you should set this to at least 2MB. That way RocksDB’s compaction is doing
    /// sequential instead of random reads
    pub compaction_readahead_size: Option<usize>,

    #[serde(default)]
    pub level_compaction_dynamic_level_bytes: bool,

    #[serde(default)]
    pub max_open_files: Option<usize>,

    #[serde(default)]
    pub log_level: LogLevelDef,

    /// See:
    /// <https://docs.rs/rocksdb/latest/rocksdb/struct.Options.html#method.set_memtable_huge_page_size>
    #[serde(default)]
    pub memtable_huge_page_size: Option<usize>,

    #[serde(
        with = "humantime_serde",
        default = "RocksSpoolParams::default_log_file_time_to_roll"
    )]
    pub log_file_time_to_roll: Duration,

    #[serde(
        with = "humantime_serde",
        default = "RocksSpoolParams::default_obsolete_files_period"
    )]
    pub obsolete_files_period: Duration,
}

impl Default for RocksSpoolParams {
    fn default() -> Self {
        Self {
            increase_parallelism: None,
            optimize_level_style_compaction: None,
            optimize_universal_style_compaction: None,
            paranoid_checks: false,
            compression_type: DBCompressionTypeDef::default(),
            compaction_readahead_size: None,
            level_compaction_dynamic_level_bytes: false,
            max_open_files: None,
            log_level: LogLevelDef::default(),
            memtable_huge_page_size: None,
            log_file_time_to_roll: Self::default_log_file_time_to_roll(),
            obsolete_files_period: Self::default_obsolete_files_period(),
        }
    }
}

impl RocksSpoolParams {
    fn default_log_file_time_to_roll() -> Duration {
        let one_day = Duration::from_secs(86400);
        one_day
    }

    fn default_obsolete_files_period() -> Duration {
        let six_hours = Duration::from_secs(6 * 60 * 60);
        six_hours
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DBCompressionTypeDef {
    None,
    Snappy,
    Zlib,
    Bz2,
    Lz4,
    Lz4hc,
    Zstd,
}

impl Into<DBCompressionType> for DBCompressionTypeDef {
    fn into(self) -> DBCompressionType {
        match self {
            Self::None => DBCompressionType::None,
            Self::Snappy => DBCompressionType::Snappy,
            Self::Zlib => DBCompressionType::Zlib,
            Self::Bz2 => DBCompressionType::Bz2,
            Self::Lz4 => DBCompressionType::Lz4,
            Self::Lz4hc => DBCompressionType::Lz4hc,
            Self::Zstd => DBCompressionType::Zstd,
        }
    }
}

impl Default for DBCompressionTypeDef {
    fn default() -> Self {
        Self::Snappy
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum LogLevelDef {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    Header,
}

impl Default for LogLevelDef {
    fn default() -> Self {
        Self::Info
    }
}

impl Into<LogLevel> for LogLevelDef {
    fn into(self) -> LogLevel {
        match self {
            Self::Debug => LogLevel::Debug,
            Self::Info => LogLevel::Info,
            Self::Warn => LogLevel::Warn,
            Self::Error => LogLevel::Error,
            Self::Fatal => LogLevel::Fatal,
            Self::Header => LogLevel::Header,
        }
    }
}

pub struct RocksSpool {
    db: Arc<DB>,
}

impl RocksSpool {
    pub fn new(path: &Path, flush: bool, params: Option<RocksSpoolParams>) -> anyhow::Result<Self> {
        let mut opts = Options::default();
        opts.set_use_fsync(flush);
        opts.create_if_missing(true);
        // The default is 1000, which is a bit high
        opts.set_keep_log_file_num(10);

        let p = params.unwrap_or_default();
        if let Some(i) = p.increase_parallelism {
            opts.increase_parallelism(i);
        }
        if let Some(i) = p.optimize_level_style_compaction {
            opts.optimize_level_style_compaction(i);
        }
        if let Some(i) = p.optimize_universal_style_compaction {
            opts.optimize_universal_style_compaction(i);
        }
        if let Some(i) = p.compaction_readahead_size {
            opts.set_compaction_readahead_size(i);
        }
        if let Some(i) = p.max_open_files {
            opts.set_max_open_files(i as _);
        }
        if let Some(i) = p.memtable_huge_page_size {
            opts.set_memtable_huge_page_size(i);
        }
        opts.set_paranoid_checks(p.paranoid_checks);
        opts.set_level_compaction_dynamic_level_bytes(p.level_compaction_dynamic_level_bytes);
        opts.set_compression_type(p.compression_type.into());
        opts.set_log_level(p.log_level.into());
        opts.set_log_file_time_to_roll(p.log_file_time_to_roll.as_secs() as usize);
        opts.set_delete_obsolete_files_period_micros(p.obsolete_files_period.as_micros() as u64);

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

    async fn store(&self, id: SpoolId, data: &[u8], force_sync: bool) -> anyhow::Result<()> {
        self.db.put(id.as_bytes(), data)?;
        if force_sync {
            let db = self.db.clone();
            tokio::task::Builder::new()
                .name("rocksdb flush")
                .spawn_blocking(move || db.flush())?
                .await??;
        }
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
                        .send(SpoolEntry::Item {
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
        let spool = RocksSpool::new(&location.path(), false, None)?;

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
            spool
                .store(id, format!("I am {i}").as_bytes(), false)
                .await?;
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
            let (tx, rx) = flume::bounded(32);
            spool.enumerate(tx)?;
            let mut count = 0;

            while let Ok(item) = rx.recv_async().await {
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
            let (tx, rx) = flume::bounded(32);
            spool.enumerate(tx)?;
            let mut unexpected = vec![];

            while let Ok(item) = rx.recv_async().await {
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
