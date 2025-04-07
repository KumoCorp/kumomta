use crate::{Spool, SpoolEntry, SpoolId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flume::Sender;
use prometheus::IntGaugeVec;
use rocksdb::perf::get_memory_usage_stats;
use rocksdb::{
    DBCompressionType, ErrorKind, IteratorMode, LogLevel, Options, WriteBatch, WriteOptions, DB,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::time::timeout_at;

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
        with = "duration_serde",
        default = "RocksSpoolParams::default_log_file_time_to_roll"
    )]
    pub log_file_time_to_roll: Duration,

    #[serde(
        with = "duration_serde",
        default = "RocksSpoolParams::default_obsolete_files_period"
    )]
    pub obsolete_files_period: Duration,

    #[serde(default)]
    pub limit_concurrent_stores: Option<usize>,
    #[serde(default)]
    pub limit_concurrent_loads: Option<usize>,
    #[serde(default)]
    pub limit_concurrent_removes: Option<usize>,
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
            limit_concurrent_stores: None,
            limit_concurrent_loads: None,
            limit_concurrent_removes: None,
        }
    }
}

impl RocksSpoolParams {
    fn default_log_file_time_to_roll() -> Duration {
        Duration::from_secs(86400)
    }

    fn default_obsolete_files_period() -> Duration {
        Duration::from_secs(6 * 60 * 60)
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

impl From<DBCompressionTypeDef> for DBCompressionType {
    fn from(val: DBCompressionTypeDef) -> Self {
        match val {
            DBCompressionTypeDef::None => DBCompressionType::None,
            DBCompressionTypeDef::Snappy => DBCompressionType::Snappy,
            DBCompressionTypeDef::Zlib => DBCompressionType::Zlib,
            DBCompressionTypeDef::Bz2 => DBCompressionType::Bz2,
            DBCompressionTypeDef::Lz4 => DBCompressionType::Lz4,
            DBCompressionTypeDef::Lz4hc => DBCompressionType::Lz4hc,
            DBCompressionTypeDef::Zstd => DBCompressionType::Zstd,
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

impl From<LogLevelDef> for LogLevel {
    fn from(val: LogLevelDef) -> Self {
        match val {
            LogLevelDef::Debug => LogLevel::Debug,
            LogLevelDef::Info => LogLevel::Info,
            LogLevelDef::Warn => LogLevel::Warn,
            LogLevelDef::Error => LogLevel::Error,
            LogLevelDef::Fatal => LogLevel::Fatal,
            LogLevelDef::Header => LogLevel::Header,
        }
    }
}

pub struct RocksSpool {
    db: Arc<DB>,
    runtime: Handle,
    limit_concurrent_stores: Option<Arc<Semaphore>>,
    limit_concurrent_loads: Option<Arc<Semaphore>>,
    limit_concurrent_removes: Option<Arc<Semaphore>>,
}

impl RocksSpool {
    pub fn new(
        path: &Path,
        flush: bool,
        params: Option<RocksSpoolParams>,
        runtime: Handle,
    ) -> anyhow::Result<Self> {
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

        let limit_concurrent_stores = p
            .limit_concurrent_stores
            .map(|n| Arc::new(Semaphore::new(n)));
        let limit_concurrent_loads = p
            .limit_concurrent_loads
            .map(|n| Arc::new(Semaphore::new(n)));
        let limit_concurrent_removes = p
            .limit_concurrent_removes
            .map(|n| Arc::new(Semaphore::new(n)));

        let db = Arc::new(DB::open(&opts, path)?);

        {
            let weak = Arc::downgrade(&db);
            tokio::spawn(metrics_monitor(weak, format!("{}", path.display())));
        }

        Ok(Self {
            db,
            runtime,
            limit_concurrent_stores,
            limit_concurrent_loads,
            limit_concurrent_removes,
        })
    }
}

#[async_trait]
impl Spool for RocksSpool {
    async fn load(&self, id: SpoolId) -> anyhow::Result<Vec<u8>> {
        let permit = match self.limit_concurrent_loads.clone() {
            Some(s) => Some(s.acquire_owned().await?),
            None => None,
        };
        let db = self.db.clone();
        tokio::task::Builder::new()
            .name("rocksdb load")
            .spawn_blocking_on(
                move || {
                    let result = db
                        .get(id.as_bytes())?
                        .ok_or_else(|| anyhow::anyhow!("no such key {id}"))?;
                    drop(permit);
                    Ok(result)
                },
                &self.runtime,
            )?
            .await?
    }

    async fn store(
        &self,
        id: SpoolId,
        data: Arc<Box<[u8]>>,
        force_sync: bool,
        deadline: Option<Instant>,
    ) -> anyhow::Result<()> {
        let mut opts = WriteOptions::default();
        opts.set_sync(force_sync);
        opts.set_no_slowdown(true);
        let mut batch = WriteBatch::default();
        batch.put(id.as_bytes(), &*data);

        match self.db.write_opt(batch, &opts) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::Incomplete => {
                let permit = match (self.limit_concurrent_stores.clone(), deadline) {
                    (Some(s), Some(deadline)) => {
                        Some(timeout_at(deadline.into(), s.acquire_owned()).await??)
                    }
                    (Some(s), None) => Some(s.acquire_owned().await?),
                    (None, _) => None,
                };
                let db = self.db.clone();
                tokio::task::Builder::new()
                    .name("rocksdb store")
                    .spawn_blocking_on(
                        move || {
                            opts.set_no_slowdown(false);
                            let mut batch = WriteBatch::default();
                            batch.put(id.as_bytes(), &*data);
                            let result = db.write_opt(batch, &opts)?;
                            drop(permit);
                            Ok(result)
                        },
                        &self.runtime,
                    )?
                    .await?
            }
            Err(err) => Err(err.into()),
        }
    }

    async fn remove(&self, id: SpoolId) -> anyhow::Result<()> {
        let mut opts = WriteOptions::default();
        opts.set_no_slowdown(true);
        let mut batch = WriteBatch::default();
        batch.delete(id.as_bytes());

        match self.db.write_opt(batch, &opts) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::Incomplete => {
                let permit = match self.limit_concurrent_removes.clone() {
                    Some(s) => Some(s.acquire_owned().await?),
                    None => None,
                };
                let db = self.db.clone();
                tokio::task::Builder::new()
                    .name("rocksdb remove")
                    .spawn_blocking_on(
                        move || {
                            opts.set_no_slowdown(false);
                            let mut batch = WriteBatch::default();
                            batch.delete(id.as_bytes());
                            let result = db.write_opt(batch, &opts)?;
                            drop(permit);
                            Ok(result)
                        },
                        &self.runtime,
                    )?
                    .await?
            }
            Err(err) => Err(err.into()),
        }
    }

    async fn cleanup(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn shutdown(&self) -> anyhow::Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || db.cancel_all_background_work(true)).await?;
        Ok(())
    }

    async fn advise_low_memory(&self) -> anyhow::Result<isize> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let usage_before = match get_memory_usage_stats(Some(&[&db]), None) {
                Ok(stats) => {
                    let stats: Stats = stats.into();
                    tracing::debug!("pre-flush: {stats:#?}");
                    stats.total()
                }
                Err(err) => {
                    tracing::error!("error getting stats: {err:#}");
                    0
                }
            };

            if let Err(err) = db.flush() {
                tracing::error!("error flushing memory: {err:#}");
            }

            let usage_after = match get_memory_usage_stats(Some(&[&db]), None) {
                Ok(stats) => {
                    let stats: Stats = stats.into();
                    tracing::debug!("post-flush: {stats:#?}");
                    stats.total()
                }
                Err(err) => {
                    tracing::error!("error getting stats: {err:#}");
                    0
                }
            };

            Ok(usage_before - usage_after)
        })
        .await?
    }

    fn enumerate(
        &self,
        sender: Sender<SpoolEntry>,
        start_time: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let db = Arc::clone(&self.db);
        tokio::task::Builder::new()
            .name("rocksdb enumerate")
            .spawn_blocking_on(
                move || {
                    let iter = db.iterator(IteratorMode::Start);
                    for entry in iter {
                        let (key, value) = entry?;
                        let id = SpoolId::from_slice(&key)
                            .ok_or_else(|| anyhow::anyhow!("invalid spool id {key:?}"))?;

                        if id.created() >= start_time {
                            // Entries created since we started must have
                            // landed there after we started and are thus
                            // not eligible for discovery via enumeration
                            continue;
                        }

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
                },
                &self.runtime,
            )?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn rocks_spool() -> anyhow::Result<()> {
        let location = tempfile::tempdir()?;
        let spool = RocksSpool::new(location.path(), false, None, Handle::current())?;

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
                .store(
                    id,
                    Arc::new(format!("I am {i}").as_bytes().to_vec().into_boxed_slice()),
                    false,
                    None,
                )
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
            spool.enumerate(tx, Utc::now())?;
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
            spool.enumerate(tx, Utc::now())?;
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

/// The rocksdb type doesn't impl Debug, so we get to do it
#[allow(unused)]
#[derive(Debug)]
struct Stats {
    pub mem_table_total: u64,
    pub mem_table_unflushed: u64,
    pub mem_table_readers_total: u64,
    pub cache_total: u64,
}

impl Stats {
    fn total(&self) -> isize {
        (self.mem_table_total + self.mem_table_readers_total + self.cache_total) as isize
    }
}

impl From<rocksdb::perf::MemoryUsageStats> for Stats {
    fn from(s: rocksdb::perf::MemoryUsageStats) -> Self {
        Self {
            mem_table_total: s.mem_table_total,
            mem_table_unflushed: s.mem_table_unflushed,
            mem_table_readers_total: s.mem_table_readers_total,
            cache_total: s.cache_total,
        }
    }
}

static MEM_TABLE_TOTAL: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "rocks_spool_mem_table_total",
        "Approximate memory usage of all the mem-tables",
        &["path"]
    )
    .unwrap()
});
static MEM_TABLE_UNFLUSHED: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "rocks_spool_mem_table_unflushed",
        "Approximate memory usage of un-flushed mem-tables",
        &["path"]
    )
    .unwrap()
});
static MEM_TABLE_READERS_TOTAL: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "rocks_spool_mem_table_readers_total",
        "Approximate memory usage of all the table readers",
        &["path"]
    )
    .unwrap()
});
static CACHE_TOTAL: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "rocks_spool_cache_total",
        "Approximate memory usage by cache",
        &["path"]
    )
    .unwrap()
});

async fn metrics_monitor(db: Weak<DB>, path: String) {
    let mem_table_total = MEM_TABLE_TOTAL
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let mem_table_unflushed = MEM_TABLE_UNFLUSHED
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let mem_table_readers_total = MEM_TABLE_READERS_TOTAL
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let cache_total = CACHE_TOTAL
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();

    loop {
        match db.upgrade() {
            Some(db) => {
                match get_memory_usage_stats(Some(&[&db]), None) {
                    Ok(stats) => {
                        mem_table_total.set(stats.mem_table_total as i64);
                        mem_table_unflushed.set(stats.mem_table_unflushed as i64);
                        mem_table_readers_total.set(stats.mem_table_readers_total as i64);
                        cache_total.set(stats.cache_total as i64);
                    }
                    Err(err) => {
                        tracing::error!("error getting stats: {err:#}");
                    }
                };
            }
            None => {
                // Dead
                return;
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
