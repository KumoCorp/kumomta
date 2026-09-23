use self::health::{Health, Policy};
use crate::{
    Spool, SpoolBackpressureTimeout, SpoolCallerDeadlineExceeded, SpoolEntry, SpoolId,
    SpoolUnhealthyError,
};
use anyhow::Context;
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
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{sleep, timeout_at};

mod health;

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

    /// Delay after an error before pausing writes, specified as a duration
    /// string such as `"15s"` or `"2m"`.
    /// Even an isolated error starts this delay. A read, write, or enumeration
    /// returning Corruption or IOError bypasses the delay and immediately
    /// pauses writes until an automatic retry or process restart. Errors in
    /// background flushes and compactions start the delay when the monitor
    /// observes their count increasing.
    #[serde(
        with = "duration_serde",
        default = "RocksSpoolParams::default_error_latch_duration"
    )]
    pub error_latch_duration: Duration,

    /// The duration to pause before an automatic retry, specified as a duration
    /// string such as `"5m"` or `"30s"`. The timer starts when the gate latches
    /// and restarts on each error observation. Must be nonzero when
    /// `allow_error_unlatch` is true. Longer pauses allow more time for
    /// operator inspection.
    #[serde(
        with = "duration_serde",
        default = "RocksSpoolParams::default_error_unlatch_duration"
    )]
    pub error_unlatch_duration: Duration,

    /// Whether writes resume automatically after `error_unlatch_duration`
    /// elapses with the gate latched and error counts unchanged. Defaults to
    /// true. Set to false to keep writes paused until an operator restarts
    /// the process after inspecting the database.
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
    /// Error observations and the gate shared by foreground operations
    /// and the monitor.
    health: Arc<Health>,
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

/// Selects the binding deadline for a backpressured write: the caller's
/// deadline when it falls before the `store_deadline` of the spool, otherwise
/// the spool deadline. Returns that deadline along with whether it was the
/// caller's.
fn select_effective_deadline(
    caller_deadline: Option<Instant>,
    spool_deadline: Instant,
) -> (Instant, bool) {
    match caller_deadline {
        Some(c) if c < spool_deadline => (c, true),
        _ => (spool_deadline, false),
    }
}

impl RocksSpool {
    /// Builds the typed error for a backpressure deadline: the
    /// caller-provided deadline when `caller_wins` is true, otherwise the
    /// spool's own `store_deadline`.
    fn timeout_err(&self, caller_wins: bool) -> anyhow::Error {
        if caller_wins {
            SpoolCallerDeadlineExceeded.into()
        } else {
            SpoolBackpressureTimeout {
                deadline: self.store_deadline,
            }
            .into()
        }
    }

    /// Routes a backpressure timeout to the delayed latch path rather than
    /// latching now: a load spike can exhaust the deadline on an intact
    /// database, and the delay waits out that transient before latching. The
    /// log fires only on the incident-starting transition (bounded to once per
    /// incident, since a latched gate rejects writes before they reach this
    /// path), not per timed-out write.
    fn note_backpressure_timeout(&self) {
        if self.health.record_foreground_error(false) {
            tracing::error!(
                "rocksdb at {}: a write timed out waiting for RocksDB to accept it. \
                 This is the first sign of trouble in a new incident; it does not by \
                 itself mean the spool has stopped accepting writes yet, but the \
                 load-shedding gate will close {:?} from now, at which point ingress \
                 will start rejecting traffic -- this happens even if no further \
                 errors occur. Investigate disk I/O and space on this host, and check \
                 the RocksDB LOG file in the spool directory for background errors.",
                self.db.path().display(),
                self.health.latch_duration(),
            );
        }
    }

    /// Acquires a concurrency permit under the effective deadline, or reports
    /// the timeout as a backpressure incident and returns the typed error.
    /// Returns `None` when `permits` is `None`.
    async fn acquire_store_permit(
        &self,
        permits: Option<Arc<Semaphore>>,
        effective_deadline: Instant,
        caller_wins: bool,
    ) -> anyhow::Result<Option<OwnedSemaphorePermit>> {
        let Some(s) = permits else {
            return Ok(None);
        };
        match timeout_at(effective_deadline.into(), s.acquire_owned()).await {
            Ok(r) => Ok(Some(r?)),
            Err(_) => {
                self.note_backpressure_timeout();
                Err(self.timeout_err(caller_wins))
            }
        }
    }

    /// Writes a RocksDB batch, retrying with exponential backoff until the
    /// write succeeds, the effective deadline is reached, the load-shedding
    /// gate latches, or RocksDB returns a non-`Incomplete` error. The write is
    /// atomic, cancellable, and doesn't hold a blocking-pool worker while it
    /// waits.
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
        if self.health.is_active() {
            return Err(SpoolUnhealthyError.into());
        }

        let mut batch = WriteBatch::default();
        apply(&mut batch);
        match self.db.write_opt(batch, &opts) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == ErrorKind::Incomplete => {}
            Err(err) => {
                record_foreground_error(&self.health, self.db.path(), &err);
                return Err(err.into());
            }
        }

        let spool_deadline = Instant::now() + self.store_deadline;
        let (effective_deadline, caller_wins) =
            select_effective_deadline(caller_deadline, spool_deadline);

        let _permit = self
            .acquire_store_permit(permits, effective_deadline, caller_wins)
            .await?;

        let mut backoff = BACKOFF_INITIAL;
        loop {
            if self.health.is_active() {
                return Err(SpoolUnhealthyError.into());
            }
            if Instant::now() >= effective_deadline {
                self.note_backpressure_timeout();
                return Err(self.timeout_err(caller_wins));
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);

            let mut batch = WriteBatch::default();
            apply(&mut batch);
            match self.db.write_opt(batch, &opts) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == ErrorKind::Incomplete => continue,
                Err(err) => {
                    record_foreground_error(&self.health, self.db.path(), &err);
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
        let policy = Policy {
            latch_duration: p.error_latch_duration,
            unlatch_duration: p.error_unlatch_duration,
            allow_unlatch: p.allow_error_unlatch,
        };
        policy.validate()?;
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

        // Ensure the directory exists so we can probe it before opening.
        // Create it the way RocksDB would (mkdir 0755, subject to umask)
        // so our pre-creation is indistinguishable from letting RocksDB
        // create it; RocksDB has no option that influences this mode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o755)
                .create(path)
                .with_context(|| format!("creating spool directory {}", path.display()))?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(path)
            .with_context(|| format!("creating spool directory {}", path.display()))?;

        // Catch a split real/effective identity meeting an over-restrictive
        // directory before RocksDB silently corrupts itself and later fails
        // with an opaque "wal_dir contains existing log file" error.
        dir_probe::probe_directory(path)
            .with_context(|| format!("spool directory {} is not usable", path.display()))?;

        let db = Arc::new(DB::open(&opts, path)?);
        // Baseline at zero: every error on this DB instance, including one
        // from startup work performed by DB::open itself, can start an
        // incident.
        let health = Arc::new(Health::new(0, policy, format!("{}", path.display())));
        let store_deadline = p.store_deadline;

        tokio::spawn(metrics_monitor(
            Arc::downgrade(&db),
            Arc::downgrade(&health),
            format!("{}", path.display()),
        ));

        Ok(Self {
            db,
            runtime,
            limit_concurrent_stores,
            limit_concurrent_loads,
            limit_concurrent_removes,
            health,
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
        let health = self.health.clone();
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
                            // Count read failures explicitly, since
                            // background-error sampling cannot detect a failed
                            // read.
                            record_foreground_error(&health, &db_path, &err);
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
        if self.health.is_active() {
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
        let health = self.health.clone();
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
                                record_foreground_error(&health, &db_path, &err);
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
    use k9::assert_equal;

    /// The caller's deadline binds only when it is sooner than the spool's own
    /// `store_deadline`. The returned flag indicates whether the caller's
    /// deadline was selected.
    #[test]
    fn effective_deadline_prefers_the_sooner_caller_deadline() {
        let base = Instant::now();
        let spool = base + Duration::from_secs(10);

        let sooner = base + Duration::from_secs(1);
        assert_equal!(
            select_effective_deadline(Some(sooner), spool),
            (sooner, true)
        );

        let later = base + Duration::from_secs(20);
        assert_equal!(
            select_effective_deadline(Some(later), spool),
            (spool, false)
        );

        assert_equal!(select_effective_deadline(None, spool), (spool, false));
    }

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

/// Returns true for errors that require an immediate write pause to protect
/// stored data from further damage.
fn is_definitively_bad(err: &rocksdb::Error) -> bool {
    matches!(err.kind(), ErrorKind::Corruption | ErrorKind::IOError)
}

/// Records a RocksDB error and pauses writes immediately for Corruption
/// and IOError. Logs the first error after startup or a retry, and any
/// error that changes the spool from accepting writes to refusing them.
fn record_foreground_error(health: &Health, path: &Path, err: &rocksdb::Error) {
    let fatal = is_definitively_bad(err);
    if health.record_foreground_error(fatal) {
        if fatal {
            tracing::error!(
                "rocksdb at {}: a store, load, or enumeration hit a {:?} error: {}. \
                 This usually means a missing or corrupt SST file. The load-shedding gate is \
                 latching immediately: ingress will reject traffic and this spool will not accept \
                 further writes until an operator investigates the RocksDB LOG file in the spool \
                 directory, repairs or restores the affected files, and either waits for automatic \
                 recovery (if allow_error_unlatch is enabled) or restarts the process.",
                path.display(),
                err.kind(),
                err.as_ref(),
            );
        } else {
            tracing::error!(
                "rocksdb at {}: a store, load, or enumeration returned an error: {}. This is the \
                 first sign of trouble in a new incident; it does not by itself mean the spool has \
                 stopped accepting writes yet, but the load-shedding gate will close {:?} from \
                 now, at which point ingress will start rejecting traffic -- this happens even if \
                 no further errors occur. Check the RocksDB LOG file in the spool directory for \
                 the underlying cause.",
                path.display(),
                err.as_ref(),
                health.latch_duration(),
            );
        }
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
/// {{since('2026.09.22-a276d4a8')}}
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
/// {{since('2026.09.22-a276d4a8')}}
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
/// Set to 1 while this spool refuses writes, or 0 otherwise. When set,
/// SMTP and HTTP ingress reject traffic, and store/remove operations
/// return an error immediately.
///
/// {{since('2026.09.22-a276d4a8')}}
///
/// A foreground operation returning `Corruption` or `IOError` immediately
/// latches the gate, causing subsequent writes to return errors. These failures
/// include missing and corrupt SST files.
///
/// Newly observed background errors, other foreground errors, and timeouts
/// while waiting for RocksDB to accept a write start the `error_latch_duration`
/// delay (default 15 seconds). Even an isolated error causes a latch after
/// this delay.
///
/// With `allow_error_unlatch = true` (the default), writes resume after
/// `error_unlatch_duration` (default 5 minutes) has elapsed since the later
/// of the latch time and the most recent error observation. If the database
/// remains damaged, another error can latch the gate again. Set
/// `allow_error_unlatch = false` to keep writes paused until an operator
/// inspects the database and restarts the process.
///
/// The monitor checks background-error growth and applies the latch and
/// retry timers, then sleeps for 5 seconds. Later errors are handled by
/// the next iteration.
///
/// Automatic retries accept this sampling delay: writes may resume between a
/// background error and its observation. Disable `allow_error_unlatch` to
/// keep writes paused across that window.
///
/// If writes remain paused, inspect `rocks_spool_background_errors` and the
/// RocksDB LOG to identify the storage failure.
static LOAD_SHED_ACTIVE: IntGaugeVec(
        "rocks_spool_load_shed_active",
        &["path"]
    );
}

declare_metric! {
/// Number of background compactions currently running for this
/// rocksdb instance.
///
/// {{since('2026.09.22-a276d4a8')}}
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
/// {{since('2026.09.22-a276d4a8')}}
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
/// {{since('2026.09.22-a276d4a8')}}
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
/// {{since('2026.09.22-a276d4a8')}}
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

async fn metrics_monitor(db: Weak<DB>, health: Weak<Health>, path: String) {
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

                let Some(health) = health.upgrade() else {
                    return;
                };
                // Apply each background sample independently. New errors
                // observed after reopening start another latch delay.
                health.sample_background_errors(bg);
                load_shed_active.set(if health.is_active() { 1 } else { 0 });
            }
            None => {
                // Dead
                return;
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
