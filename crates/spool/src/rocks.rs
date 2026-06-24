use crate::{
    Spool, SpoolBackpressureTimeout, SpoolCallerDeadlineExceeded, SpoolEntry, SpoolId,
    SpoolUnhealthyError,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flume::Sender;
use kumo_prometheus::declare_metric;
use rocksdb::perf::get_memory_usage_stats;
use rocksdb::properties::{
    ACTUAL_DELAYED_WRITE_RATE, BACKGROUND_ERRORS, COMPACTION_PENDING,
    ESTIMATE_PENDING_COMPACTION_BYTES, IS_WRITE_STOPPED, NUM_RUNNING_COMPACTIONS,
};
use rocksdb::{
    BottommostLevelCompaction, CompactOptions, DBCompressionType, ErrorKind, IteratorMode,
    LogLevel, Options, WaitForCompactOptions, WriteBatch, WriteOptions, DB,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::time::{sleep, timeout_at};

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

    /// Size in bytes of the rocksdb memtable that buffers writes before
    /// being flushed to disk as a new SST file.
    ///
    /// Smaller values produce smaller, more frequent SST files and
    /// trigger compactions sooner -- useful in test setups that need
    /// to force the storage through its full write/compact lifecycle
    /// quickly.  Larger values amortize compaction overhead but
    /// increase memory use and recovery time after restart.  Leave
    /// unset to use the rocksdb default.
    #[serde(default)]
    pub write_buffer_size: Option<usize>,

    /// Number of level-0 SST files at which rocksdb will stop
    /// accepting writes.  Lower values transition the database into
    /// the write-stopped state more quickly when background
    /// compaction cannot keep up, which is useful for tests that
    /// need to deterministically observe that condition.  Leave
    /// unset to use the rocksdb default.
    #[serde(default)]
    pub level0_stop_writes_trigger: Option<i32>,

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

    /// Upper bound on the wait that `store()` and `remove()` will
    /// tolerate when rocksdb is applying backpressure.  Callers may
    /// provide a shorter deadline (typically derived from an SMTP
    /// client's idle timeout); the effective deadline is the minimum
    /// of the two.  Going longer than the caller-provided value risks
    /// the client timing out and retrying, which would produce
    /// duplicate deliveries -- this option therefore only narrows the
    /// effective deadline, it never extends it.
    #[serde(
        with = "duration_serde",
        default = "RocksSpoolParams::default_store_deadline"
    )]
    pub store_deadline: Duration,

    /// How long the composite "this database is wedged" signal must
    /// hold continuously before the load-shedding gate latches.
    /// The signal goes high whenever the rocksdb
    /// `background-errors` counter has grown above the value
    /// observed at process start, or any foreground spool operation
    /// has returned a rocksdb error since process start.  Brief
    /// blips that recover within this window do not latch the gate.
    #[serde(
        with = "duration_serde",
        default = "RocksSpoolParams::default_error_latch_duration"
    )]
    pub error_latch_duration: Duration,

    /// How long the healthy state must hold continuously before the
    /// load-shedding gate auto-unlatches.  Only consulted when
    /// `allow_error_unlatch` is true.  A relatively long value
    /// (minutes) gives operators time to inspect the database after a
    /// brief failure window before the daemon starts accepting writes
    /// again on its own.
    #[serde(
        with = "duration_serde",
        default = "RocksSpoolParams::default_error_unlatch_duration"
    )]
    pub error_unlatch_duration: Duration,

    /// When true (the default), the load-shedding gate clears itself
    /// after `error_unlatch_duration` of observed recovery.  Set to
    /// false to require an operator restart to clear the gate, which
    /// is appropriate when you want a human to confirm the underlying
    /// cause is resolved before accepting traffic again.
    #[serde(default = "RocksSpoolParams::default_allow_error_unlatch")]
    pub allow_error_unlatch: bool,
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
            write_buffer_size: None,
            level0_stop_writes_trigger: None,
            log_level: LogLevelDef::default(),
            memtable_huge_page_size: None,
            log_file_time_to_roll: Self::default_log_file_time_to_roll(),
            obsolete_files_period: Self::default_obsolete_files_period(),
            limit_concurrent_stores: None,
            limit_concurrent_loads: None,
            limit_concurrent_removes: None,
            store_deadline: Self::default_store_deadline(),
            error_latch_duration: Self::default_error_latch_duration(),
            error_unlatch_duration: Self::default_error_unlatch_duration(),
            allow_error_unlatch: Self::default_allow_error_unlatch(),
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

    fn default_store_deadline() -> Duration {
        Duration::from_secs(30)
    }

    fn default_error_latch_duration() -> Duration {
        Duration::from_secs(15)
    }

    fn default_error_unlatch_duration() -> Duration {
        Duration::from_secs(5 * 60)
    }

    fn default_allow_error_unlatch() -> bool {
        true
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
    /// Latched load-shedding gate driven by `metrics_monitor`.  When
    /// set, foreground `store()`/`remove()` calls return an error
    /// instead of waiting on rocksdb backpressure, and the ingress
    /// paths refuse new traffic.  See `metrics_monitor` for the
    /// composite signal that drives latch/unlatch transitions.
    load_shed_active: Arc<AtomicBool>,
    /// Count of foreground spool operations (load, enumerate,
    /// store, remove) that have failed with a rocksdb error
    /// since the last auto-unlatch (or since process start if no
    /// auto-unlatch has happened).  `metrics_monitor` reads this
    /// to detect failure modes that do not surface as background
    /// errors -- notably a missing SST discovered during a
    /// `get()`, which the C++ side reports to the caller but does
    /// not feed into `rocksdb.background-errors`.  Reset to 0 by
    /// the auto-unlatch path's compare-exchange so a subsequent
    /// blip can be observed as fresh growth.
    foreground_errors: Arc<AtomicU64>,
    store_deadline: Duration,
}

/// Initial sleep interval for the `store`/`remove` backpressure loop.
/// Chosen low enough that brief, sub-millisecond memtable backpressure
/// is caught with negligible added latency on the slow path.
const BACKOFF_INITIAL: Duration = Duration::from_micros(500);
/// Upper bound on the backpressure loop sleep interval.  Keeps the
/// load-shedding gate observable within a bounded window even during
/// a long wedge.
const BACKOFF_MAX: Duration = Duration::from_millis(50);

impl RocksSpool {
    /// Driver shared by `store()` and `remove()`.  Issues `write_opt`
    /// with `no_slowdown=true` repeatedly with exponential backoff
    /// until one of: the write succeeds, the effective deadline is
    /// reached, the load-shedding gate latches, or rocksdb returns a
    /// non-`Incomplete` error.
    ///
    /// This replaces an earlier design that used `spawn_blocking` with
    /// `no_slowdown=false`.  That approach could not be cancelled, held
    /// a blocking-pool worker per stalled call (risking pool
    /// exhaustion during a wedge), and could not observe the latched
    /// load-shedding gate.  The polling design preserves write
    /// atomicity -- each iteration is a single rocksdb batch write,
    /// which is atomic by construction -- while restoring
    /// cancellation, gate observability, and bounded resource use.
    async fn write_with_backpressure(
        &self,
        opts: WriteOptions,
        caller_deadline: Option<Instant>,
        permits: Option<Arc<Semaphore>>,
        apply: impl Fn(&mut WriteBatch),
    ) -> anyhow::Result<()> {
        // Gate at the top so that the load-shedding mirror affects
        // every store, not just those that happen to hit backpressure.
        // A relaxed atomic load is essentially free compared to the
        // rocksdb FFI write below; this preserves the healthy hot
        // path's latency profile while giving the gate consistent
        // semantics across the in-flight call sites that aren't
        // covered by the per-connection ingress checks (notably,
        // already-established SMTP connections doing new
        // transactions).
        if self.load_shed_active.load(Ordering::Relaxed) {
            return Err(SpoolUnhealthyError.into());
        }

        let mut batch = WriteBatch::default();
        apply(&mut batch);
        match self.db.write_opt(batch, &opts) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == ErrorKind::Incomplete => {}
            Err(err) => {
                record_foreground_error(
                    &self.foreground_errors,
                    &self.load_shed_active,
                    self.db.path(),
                    &err,
                );
                return Err(err.into());
            }
        }

        let spool_deadline = Instant::now() + self.store_deadline;
        // Decide upfront which side's deadline wins, so both the
        // semaphore-acquisition timeout and the backpressure-loop
        // timeout can surface the matching typed error.  Without
        // this, the SMTP layer cannot tell a caller-provided
        // `data_processing_timeout` from the spool's own
        // `store_deadline` and would mis-label the wire response.
        let (effective_deadline, caller_wins) = match caller_deadline {
            Some(c) if c < spool_deadline => (c, true),
            _ => (spool_deadline, false),
        };
        let timeout_err = || -> anyhow::Error {
            if caller_wins {
                SpoolCallerDeadlineExceeded.into()
            } else {
                SpoolBackpressureTimeout {
                    deadline: self.store_deadline,
                }
                .into()
            }
        };

        let _permit = match permits {
            Some(s) => match timeout_at(effective_deadline.into(), s.acquire_owned()).await {
                Ok(r) => Some(r?),
                Err(_) => return Err(timeout_err()),
            },
            None => None,
        };

        let mut backoff = BACKOFF_INITIAL;
        loop {
            if self.load_shed_active.load(Ordering::Relaxed) {
                return Err(SpoolUnhealthyError.into());
            }
            if Instant::now() >= effective_deadline {
                // Sustained backpressure for the full deadline is
                // itself a useful signal that the spool may be
                // unhealthy.  Feed it into the foreground error
                // machinery so the debounced latch path can react if
                // we see this repeatedly; an occasional one-off
                // (e.g. a brief load spike) gets washed out by the
                // `error_latch_duration` window.  We do not
                // immediate-latch because no rocksdb error has been
                // returned -- the inability to make progress is
                // ambiguous, not definitively bad.
                self.foreground_errors.fetch_add(1, Ordering::Relaxed);
                return Err(timeout_err());
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);

            let mut batch = WriteBatch::default();
            apply(&mut batch);
            match self.db.write_opt(batch, &opts) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == ErrorKind::Incomplete => continue,
                Err(err) => {
                    record_foreground_error(
                        &self.foreground_errors,
                        &self.load_shed_active,
                        self.db.path(),
                        &err,
                    );
                    return Err(err.into());
                }
            }
        }
    }

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
        if let Some(i) = p.write_buffer_size {
            opts.set_write_buffer_size(i);
        }
        if let Some(i) = p.level0_stop_writes_trigger {
            opts.set_level_zero_stop_writes_trigger(i);
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
        let load_shed_active = Arc::new(AtomicBool::new(false));
        let foreground_errors = Arc::new(AtomicU64::new(0));
        let store_deadline = p.store_deadline;

        {
            let weak_db = Arc::downgrade(&db);
            let weak_mirror = Arc::downgrade(&load_shed_active);
            let weak_fg_errors = Arc::downgrade(&foreground_errors);
            tokio::spawn(metrics_monitor(
                weak_db,
                weak_mirror,
                weak_fg_errors,
                format!("{}", path.display()),
                p.error_latch_duration,
                p.error_unlatch_duration,
                p.allow_error_unlatch,
            ));
        }

        Ok(Self {
            db,
            runtime,
            limit_concurrent_stores,
            limit_concurrent_loads,
            limit_concurrent_removes,
            load_shed_active,
            foreground_errors,
            store_deadline,
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
        let fg_errors = self.foreground_errors.clone();
        let load_shed = self.load_shed_active.clone();
        let db_path: PathBuf = self.db.path().to_owned();
        tokio::task::Builder::new()
            .name("rocksdb load")
            .spawn_blocking_on(
                move || {
                    let result = match db.get(id.as_bytes()) {
                        Ok(Some(v)) => v,
                        Ok(None) => {
                            drop(permit);
                            anyhow::bail!("no such key {id}");
                        }
                        Err(err) => {
                            // Rocksdb get errors (e.g. a missing SST
                            // file discovered during the read) do not
                            // increment rocksdb.background-errors.
                            // Record them so the load-shedding gate
                            // can react -- immediately for fatal
                            // classes, or after debounce otherwise.
                            record_foreground_error(&fg_errors, &load_shed, &db_path, &err);
                            drop(permit);
                            return Err(err.into());
                        }
                    };
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

        self.write_with_backpressure(
            opts,
            deadline,
            self.limit_concurrent_stores.clone(),
            |batch| batch.put(id.as_bytes(), &*data),
        )
        .await
    }

    async fn remove(&self, id: SpoolId) -> anyhow::Result<()> {
        let mut opts = WriteOptions::default();
        opts.set_no_slowdown(true);

        self.write_with_backpressure(opts, None, self.limit_concurrent_removes.clone(), |batch| {
            batch.delete(id.as_bytes())
        })
        .await
    }

    async fn cleanup(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn compact(&self) -> anyhow::Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            db.flush()?;
            // Force bottommost-level compaction so the entire keyspace
            // is rewritten; without this, single-level layouts cause
            // the call to be a no-op even when there are missing files
            // that we'd want to surface as errors.
            let mut opts = CompactOptions::default();
            opts.set_bottommost_level_compaction(BottommostLevelCompaction::Force);
            opts.set_exclusive_manual_compaction(true);
            db.compact_range_opt::<&[u8], &[u8]>(None, None, &opts);
            // compact_range itself does not return errors -- wait_for_compact
            // does, and is what surfaces background failures (e.g. a
            // missing SST encountered during compaction) to the caller.
            let wait_opts = WaitForCompactOptions::default();
            db.wait_for_compact(&wait_opts)?;
            Ok(())
        })
        .await?
    }

    async fn shutdown(&self) -> anyhow::Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || db.cancel_all_background_work(true)).await?;
        Ok(())
    }

    fn unhealthy_reason(&self) -> Option<&'static str> {
        if self.load_shed_active.load(Ordering::Relaxed) {
            Some("the spool is not accepting writes")
        } else {
            None
        }
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
        let fg_errors = self.foreground_errors.clone();
        let load_shed = self.load_shed_active.clone();
        let db_path: PathBuf = self.db.path().to_owned();
        tokio::task::Builder::new()
            .name("rocksdb enumerate")
            .spawn_blocking_on(
                move || {
                    let iter = db.iterator(IteratorMode::Start);
                    for entry in iter {
                        let (key, value) = match entry {
                            Ok(e) => e,
                            Err(err) => {
                                // Iterator errors typically indicate a
                                // missing or corrupt SST file
                                // discovered while walking the
                                // keyspace.  Feed into the foreground
                                // error machinery so the gate latches
                                // (immediately for IOError /
                                // Corruption) and abort the
                                // enumeration.
                                record_foreground_error(&fg_errors, &load_shed, &db_path, &err);
                                return Err(err.into());
                            }
                        };
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

/// Read an integer-valued rocksdb property, returning 0 if the property
/// is missing or cannot be parsed.  Used for hot-path checks and metrics
/// gathering; callers that want to distinguish "missing" from "zero"
/// should call `property_int_value` directly.
fn property_u64(db: &DB, name: &rocksdb::properties::PropName) -> u64 {
    db.property_int_value(name).ok().flatten().unwrap_or(0)
}

/// Classify a rocksdb error returned from a foreground spool
/// operation.  `Corruption` and `IOError` are returned by rocksdb
/// when the underlying database state is observably wrong (a missing
/// or corrupt SST file, a checksum mismatch, etc.) -- conditions
/// that do not have any transient interpretation.  We use this to
/// latch the load-shedding gate immediately rather than waiting out
/// the normal debounce window.
fn is_definitively_bad(err: &rocksdb::Error) -> bool {
    matches!(err.kind(), ErrorKind::Corruption | ErrorKind::IOError)
}

/// Record a foreground spool error: always increment the counter so
/// the metrics monitor can observe sustained-failure patterns, and
/// for errors classified as definitively bad, latch the gate now.
/// Logs once on the false-to-true transition of the gate.
fn record_foreground_error(
    fg_errors: &AtomicU64,
    load_shed: &AtomicBool,
    path: &Path,
    err: &rocksdb::Error,
) {
    fg_errors.fetch_add(1, Ordering::Relaxed);
    if is_definitively_bad(err) && !load_shed.swap(true, Ordering::Relaxed) {
        tracing::error!(
            "rocksdb at {}: fatal foreground error ({:?}); load-shedding \
             gate latched immediately. Underlying error: {}",
            path.display(),
            err.kind(),
            err.as_ref(),
        );
    }
}

declare_metric! {
/// Approximate memory usage (bytes) of all the mem-tables.
///
/// This may be useful when understanding the memory usage of
/// the system.
static MEM_TABLE_TOTAL: IntGaugeVec(
        "rocks_spool_mem_table_total",
        &["path"]
    );
}

declare_metric! {
/// Approximate memory usage (bytes) of un-flushed mem-tables.
///
/// This may be useful when understanding the memory usage of
/// the system.
static MEM_TABLE_UNFLUSHED: IntGaugeVec(
        "rocks_spool_mem_table_unflushed",
        &["path"]
    );
}

declare_metric! {
/// Approximate memory usage (bytes) of all the table readers.
///
/// This may be useful when understanding the memory usage of
/// the system.
static MEM_TABLE_READERS_TOTAL: IntGaugeVec(
        "rocks_spool_mem_table_readers_total",
        &["path"]
    );
}

declare_metric! {
/// Approximate memory (bytes) usage by cache.
///
/// This may be useful when understanding the memory usage of
/// the system.
static CACHE_TOTAL: IntGaugeVec(
        "rocks_spool_cache_total",
        &["path"]
    );
}

declare_metric! {
/// Accumulated count of background errors encountered by the rocksdb
/// instance (failed flushes or compactions, typically caused by I/O
/// errors such as missing or corrupt SST files, ENOSPC, or permission
/// problems).
///
/// {{since('dev')}}
///
/// This counter is **monotonic** for the lifetime of the process: it
/// does not decrease when rocksdb auto-resumes from transient errors
/// such as a brief ENOSPC.  A non-zero value therefore does not
/// necessarily mean the database is currently wedged; it means at
/// least one background error has occurred since the process started.
///
/// For SRE monitoring, alert on the **rate of change** (e.g.
/// `increase(rocks_spool_background_errors[5m]) > 0`) to catch new
/// occurrences.  For the actionable "the database is wedged right
/// now and we are shedding load" signal, page on
/// `rocks_spool_load_shed_active` instead, which combines this
/// counter, foreground read/write errors, and rocksdb error
/// severity into a single latched indicator.
static BACKGROUND_ERRORS_METRIC: IntGaugeVec(
        "rocks_spool_background_errors",
        &["path"]
    );
}

declare_metric! {
/// Set to 1 when the rocksdb instance is currently refusing writes
/// at the WriteController layer (memtable count or L0 file count
/// reached the stop threshold), 0 otherwise.
///
/// {{since('dev')}}
///
/// This reflects rocksdb's own `is-write-stopped` property and
/// indicates backpressure rather than a fatal background error.
/// Healthy databases under bursty load may briefly report 1 here.
/// For the "the database is wedged due to a background error"
/// signal, see `rocks_spool_load_shed_active` instead.
static WRITE_STOPPED: IntGaugeVec(
        "rocks_spool_write_stopped",
        &["path"]
    );
}

declare_metric! {
/// Set to 1 when this spool's load-shedding gate is latched, 0
/// otherwise.  When set, ingress paths (SMTP, HTTP inject) reject
/// traffic and foreground store/remove operations fail fast rather
/// than stall.
///
/// {{since('dev')}}
///
/// The gate latches in either of two ways:
///
/// * **Immediate**: a foreground spool operation (load, store,
///   remove) returns a rocksdb error classified as definitively
///   bad (`Corruption` or `IOError` -- e.g. a missing or corrupt
///   SST file discovered during a read).  These conditions have
///   no transient interpretation, so the gate latches on the
///   first such observation.
/// * **Debounced**: less specific failure signals --
///   `background-errors` has grown since this process started, or
///   foreground operations have returned non-fatal errors --
///   sustained continuously for the configured
///   `error_latch_duration` (default 15s).  This filters out
///   brief auto-resumed errors.
///
/// If `allow_error_unlatch` is enabled (the default), the gate
/// auto-clears after `error_unlatch_duration` of observed recovery
/// (default 5 minutes) with no new errors of either class.
/// Otherwise it stays set until the process is restarted.
///
/// SREs should treat any sustained non-zero value as an
/// operator-actionable incident; pair this metric with
/// `rocks_spool_background_errors` to understand why.
static LOAD_SHED_ACTIVE: IntGaugeVec(
        "rocks_spool_load_shed_active",
        &["path"]
    );
}

declare_metric! {
/// Number of background compactions currently running for this
/// rocksdb instance.
///
/// {{since('dev')}}
///
/// In a healthy, actively-written spool this is typically non-zero
/// in bursts.  A value persistently stuck at 0 while
/// `rocks_spool_compaction_pending` or
/// `rocks_spool_estimate_pending_compaction_bytes` is growing is a
/// strong indicator that the background worker is wedged --
/// cross-reference `rocks_spool_write_stopped` and
/// `rocks_spool_background_errors`.
static NUM_RUNNING_COMPACTIONS_METRIC: IntGaugeVec(
        "rocks_spool_num_running_compactions",
        &["path"]
    );
}

declare_metric! {
/// Set to 1 when at least one compaction is pending for this rocksdb
/// instance, 0 otherwise.
///
/// {{since('dev')}}
///
/// Brief flapping is normal under write load.  A value of 1 that
/// persists alongside `rocks_spool_num_running_compactions == 0` is
/// suspicious and suggests the compaction worker is not making
/// progress.
static COMPACTION_PENDING_METRIC: IntGaugeVec(
        "rocks_spool_compaction_pending",
        &["path"]
    );
}

declare_metric! {
/// Estimated total bytes that compaction needs to rewrite to bring
/// all levels back under their target sizes.
///
/// {{since('dev')}}
///
/// This is a backlog indicator.  Steady-state values depend heavily
/// on write rate, compression, and the configured compaction style,
/// so absolute thresholds should be derived from each deployment's
/// baseline.  Unbounded growth over a multi-hour window indicates
/// that compaction cannot keep up with the write rate, which
/// eventually leads to write slowdown
/// (`rocks_spool_actual_delayed_write_rate` becomes non-zero) and
/// then to write stop (`rocks_spool_write_stopped` becomes 1).
///
/// Only meaningful for level-style compaction.
static ESTIMATE_PENDING_COMPACTION_BYTES_METRIC: IntGaugeVec(
        "rocks_spool_estimate_pending_compaction_bytes",
        &["path"]
    );
}

declare_metric! {
/// Current delayed write rate (bytes/second) applied by rocksdb to
/// throttle foreground writers.  0 means no slowdown is in effect.
///
/// {{since('dev')}}
///
/// A non-zero value means rocksdb is intentionally slowing writers
/// down because compaction or flush is falling behind.  This is the
/// early-warning signal that precedes a full write stop: if this
/// remains non-zero for an extended period, investigate the
/// compaction backlog
/// (`rocks_spool_estimate_pending_compaction_bytes`) and underlying
/// disk throughput before the database transitions to
/// `rocks_spool_write_stopped == 1`.
static ACTUAL_DELAYED_WRITE_RATE_METRIC: IntGaugeVec(
        "rocks_spool_actual_delayed_write_rate",
        &["path"]
    );
}

/// Internal state for the load-shedding latch state machine.  See the
/// per-tick logic in `metrics_monitor`.
struct HealthState {
    /// `background-errors` count observed on the first monitor tick.
    /// Only growth above this baseline counts toward latching, so a
    /// daemon restarted against a DB whose accumulated count is
    /// already non-zero does not immediately latch.
    initial_bg_errors: u64,
    /// `background-errors` count from the previous monitor tick.
    /// Used both for once-per-transition logging and to detect
    /// quiet windows when deciding whether to auto-unlatch.
    prev_bg_errors: u64,
    /// Foreground spool error count from the previous monitor tick.
    /// The counter itself starts at zero per process, so unlike
    /// `initial_bg_errors` there is no separate baseline -- any
    /// non-zero observation reflects errors in the current run.
    prev_fg_errors: u64,
    /// Instant we first observed an unhealthy signal (bg above
    /// baseline OR any foreground errors) in the current run.
    unhealthy_since: Option<Instant>,
    /// Instant of the most recent monitor tick where bg_errors
    /// increased over the previous tick.
    last_bg_growth_at: Option<Instant>,
    /// Instant of the most recent monitor tick where the
    /// foreground error counter increased over the previous tick.
    last_fg_growth_at: Option<Instant>,
    latched: bool,
}

async fn metrics_monitor(
    db: Weak<DB>,
    mirror: Weak<AtomicBool>,
    foreground_errors: Weak<AtomicU64>,
    path: String,
    latch_duration: Duration,
    unlatch_duration: Duration,
    allow_unlatch: bool,
) {
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
    let background_errors = BACKGROUND_ERRORS_METRIC
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let write_stopped = WRITE_STOPPED
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let load_shed_active = LOAD_SHED_ACTIVE
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let num_running_compactions = NUM_RUNNING_COMPACTIONS_METRIC
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let compaction_pending = COMPACTION_PENDING_METRIC
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let estimate_pending_compaction_bytes = ESTIMATE_PENDING_COMPACTION_BYTES_METRIC
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();
    let actual_delayed_write_rate = ACTUAL_DELAYED_WRITE_RATE_METRIC
        .get_metric_with_label_values(&[path.as_str()])
        .unwrap();

    // Initial bg_errors observation anchors the latch logic against
    // pre-existing accumulated errors from prior process lifetimes,
    // so a restart against a DB with a historical count does not
    // immediately latch.  If the DB has already been dropped before
    // we get here, exit silently -- there is nothing to monitor.
    let Some(db_init) = db.upgrade() else {
        return;
    };
    let initial_bg = property_u64(&db_init, BACKGROUND_ERRORS);
    drop(db_init);
    let mut state = HealthState {
        initial_bg_errors: initial_bg,
        prev_bg_errors: initial_bg,
        prev_fg_errors: 0,
        unhealthy_since: None,
        last_bg_growth_at: None,
        last_fg_growth_at: None,
        latched: false,
    };

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

                let bg = property_u64(&db, BACKGROUND_ERRORS);
                let stopped = property_u64(&db, IS_WRITE_STOPPED);
                let compaction_pending_now = property_u64(&db, COMPACTION_PENDING);
                let num_running = property_u64(&db, NUM_RUNNING_COMPACTIONS);
                background_errors.set(bg as i64);
                write_stopped.set(stopped as i64);
                num_running_compactions.set(num_running as i64);
                compaction_pending.set(compaction_pending_now as i64);
                estimate_pending_compaction_bytes
                    .set(property_u64(&db, ESTIMATE_PENDING_COMPACTION_BYTES) as i64);
                actual_delayed_write_rate.set(property_u64(&db, ACTUAL_DELAYED_WRITE_RATE) as i64);

                let now = Instant::now();
                let fg = foreground_errors
                    .upgrade()
                    .map(|c| c.load(Ordering::Relaxed))
                    .unwrap_or(0);

                // The foreground error path may have latched the gate
                // directly on a fatal error (Corruption/IOError) since
                // our last tick.  Reconcile our internal state so the
                // unlatch logic sees the gate as latched without
                // duplicating the "gate latched" log.
                if let Some(m) = mirror.upgrade() {
                    if m.load(Ordering::Relaxed) && !state.latched {
                        state.latched = true;
                        state.unhealthy_since = Some(now);
                    }
                }

                if bg > state.prev_bg_errors {
                    tracing::error!(
                        "rocksdb at {path}: background error count \
                         increased from {prev} to {bg}; check the LOG \
                         file in that directory for details",
                        prev = state.prev_bg_errors,
                    );
                    state.last_bg_growth_at = Some(now);
                }
                state.prev_bg_errors = bg;

                if fg > state.prev_fg_errors {
                    tracing::error!(
                        "rocksdb at {path}: foreground spool error count \
                         increased from {prev} to {fg}; this typically \
                         indicates a missing or corrupt SST file \
                         discovered during a read",
                        prev = state.prev_fg_errors,
                    );
                    state.last_fg_growth_at = Some(now);
                }
                state.prev_fg_errors = fg;

                // Latch signal: background errors have grown since this
                // process started, OR any foreground errors have been
                // observed.  We cannot use rocksdb's
                // `compaction-pending` or `is-write-stopped` properties
                // to refine this -- when paranoid_checks fires,
                // rocksdb pauses background scheduling and both
                // properties drop to 0 even though the DB is wedged.
                // The sustained-for-latch_duration window is what
                // filters out brief auto-resumed blips.
                let unhealthy_now = bg > state.initial_bg_errors || fg > 0;

                if unhealthy_now {
                    let since = *state.unhealthy_since.get_or_insert(now);
                    if !state.latched && now.duration_since(since) >= latch_duration {
                        state.latched = true;
                        if let Some(m) = mirror.upgrade() {
                            m.store(true, Ordering::Relaxed);
                        }
                        tracing::error!(
                            "rocksdb at {path}: load-shedding gate latched \
                             after {latch_duration:?} of sustained background \
                             errors (accumulated count: {bg}). Ingress paths \
                             will now reject traffic. Inspect the LOG file \
                             for the underlying cause.",
                        );
                    }
                } else if !state.latched {
                    // Healthy tick before latch: reset the debounce
                    // window so a future blip gets its full
                    // latch_duration grace, not a stale baseline
                    // from an earlier, separate transient.
                    state.unhealthy_since = None;
                }

                // Auto-unlatch: neither bg_errors nor fg_errors have
                // grown for `unlatch_duration`.  This catches
                // self-healed transients (one blip, then quiet).  It
                // does NOT distinguish a self-healed transient from a
                // truly wedged DB where compactions have been
                // abandoned and simply stopped producing further
                // errors.  Operators who require a stronger
                // guarantee should set `allow_error_unlatch = false`.
                if state.latched && allow_unlatch {
                    let bg_stable_since = state.last_bg_growth_at.unwrap_or(now);
                    let fg_stable_since = state.last_fg_growth_at.unwrap_or(now);
                    let stable_since = bg_stable_since.max(fg_stable_since);
                    if now.duration_since(stable_since) >= unlatch_duration {
                        // CAS the foreground counter from our tick
                        // snapshot to 0.  This is the synchronization
                        // point that prevents an auto-unlatch from
                        // racing with a concurrent
                        // record_foreground_error: if a fresh fatal
                        // error landed mid-tick, the CAS fails and we
                        // defer the unlatch to the next tick.
                        let cleared = match foreground_errors.upgrade() {
                            Some(c) => c
                                .compare_exchange(fg, 0, Ordering::Relaxed, Ordering::Relaxed)
                                .is_ok(),
                            // Process is shutting down; skip.
                            None => false,
                        };
                        if cleared {
                            state.latched = false;
                            // Re-anchor the baselines at the current
                            // counts so that we only re-latch on *new*
                            // growth above this point; otherwise the
                            // static post-transient counts would keep us
                            // permanently unhealthy.
                            state.initial_bg_errors = bg;
                            state.prev_fg_errors = 0;
                            state.unhealthy_since = None;
                            if let Some(m) = mirror.upgrade() {
                                m.store(false, Ordering::Relaxed);
                            }
                            tracing::info!(
                                "rocksdb at {path}: load-shedding gate cleared \
                                 after {unlatch_duration:?} without new errors \
                                 (bg baseline re-anchored at {bg}); ingress \
                                 paths will accept traffic again",
                            );
                        }
                    }
                }
                load_shed_active.set(if state.latched { 1 } else { 0 });
            }
            None => {
                // Dead
                return;
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
