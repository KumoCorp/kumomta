use crate::logging::{LogCommand, LogDirTimeZone, LogDirectory, LogRecordParams};
use anyhow::Context;
use chrono::FixedOffset;
use flume::Receiver;
pub use kumo_log_types::*;
use kumo_server_common::disk_space::MinFree;
use kumo_server_common::log::{mark_existing_logs_as_done_in_dir, OpenedFile};
use kumo_server_memory::subscribe_to_memory_status_changes_async;
use kumo_template::{Template, TemplateEngine};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zstd::stream::write::Encoder;

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LogFileParams {
    /// Where to place the log files
    pub log_dir: LogDirectory,
    /// Which timezone to use for resolving dynamic log_dir templates
    #[serde(default)]
    pub log_dir_timezone: LogDirTimeZone,
    /// How many uncompressed bytes to allow per file segment
    #[serde(default = "LogFileParams::default_max_file_size")]
    pub max_file_size: u64,
    /// Maximum number of outstanding items to be logged before
    /// the submission will block; helps to avoid runaway issues
    /// spiralling out of control.
    #[serde(default = "LogFileParams::default_back_pressure")]
    pub back_pressure: usize,

    /// The level of compression.
    /// 0 - use the zstd default level (probably 3).
    /// 1-21 are the explicitly configurable levels
    #[serde(default = "LogFileParams::default_compression_level")]
    pub compression_level: i32,

    #[serde(default, with = "duration_serde")]
    pub max_segment_duration: Option<Duration>,

    /// List of meta fields to capture in the log
    #[serde(default)]
    pub meta: Vec<String>,

    /// List of message headers to capture in the log
    #[serde(default)]
    pub headers: Vec<String>,

    #[serde(default)]
    pub per_record: HashMap<RecordType, LogRecordParams>,

    /// The name of an event which can be used to filter
    /// out log records which should not be logged to this
    /// log file
    #[serde(default)]
    pub filter_event: Option<String>,

    #[serde(default)]
    pub min_free_space: MinFree,
    #[serde(default)]
    pub min_free_inodes: MinFree,
}

impl LogFileParams {
    pub fn default_max_file_size() -> u64 {
        1_000_000_000
    }
    pub fn default_back_pressure() -> usize {
        128_000
    }
    pub fn default_compression_level() -> i32 {
        0 // use the zstd default
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct FileNameKey {
    dir_id: usize,
    log_dir: PathBuf,
    suffix: Option<String>,
}

pub struct LogThreadState {
    pub params: LogFileParams,
    pub receiver: Receiver<LogCommand>,
    pub template_engine: TemplateEngine,
    pub file_map: HashMap<FileNameKey, OpenedFile>,
}

impl LogThreadState {
    pub async fn logger_thread(&mut self) {
        tracing::debug!("LogFileParams: {:#?}", self.params);

        if !config::is_validating() {
            self.mark_existing_logs_as_done();
        }
        let mut expire_counter = 0u16;

        let mut memory_status = subscribe_to_memory_status_changes_async().await;

        loop {
            let deadline = self.get_deadline();
            tracing::debug!("waiting until deadline={deadline:?} for a log record");

            let cmd = if let Some(deadline) = deadline {
                tokio::select! {
                    cmd = self.receiver.recv_async() => {
                        // If we are super busy and always have another command immediately
                        // available, we need to take the opportunity to notice when
                        // segments expire.
                        // We check every so many events so that we're not spending excessive
                        // amounts of processing considering this.
                        // When we go idle, we'll take the other branch and notice.
                        expire_counter = expire_counter.wrapping_add(1);
                        if expire_counter == 10_000 {
                            self.expire();
                            expire_counter = 0;
                        }
                        cmd
                    }
                    _ = memory_status.changed() => {
                        if kumo_server_memory::get_headroom() == 0 {
                            tracing::debug!("memory is short, flushing open logs");
                            self.file_map.clear();
                        }

                        continue;
                    }
                    _ = tokio::time::sleep_until(deadline.into()) => {
                        tracing::debug!("deadline reached, running expiration for this segment");
                        self.expire();
                        continue;
                    }
                }
            } else {
                self.receiver.recv_async().await
            };
            let cmd = match cmd {
                Ok(cmd) => cmd,
                other => {
                    tracing::debug!("logging channel closed {other:?}");
                    return;
                }
            };
            match cmd {
                LogCommand::Terminate => {
                    tracing::debug!("LogCommand::Terminate received. Stopping writing logs");
                    break;
                }
                LogCommand::Record(record, _msg) => {
                    if let Err(err) = self.do_record(record) {
                        tracing::error!("failed to log: {err:#}");
                    };
                }
            }
        }

        tracing::debug!("Clearing any buffered files prior to completion");
        self.file_map.clear();
    }

    fn mark_existing_logs_as_done(&self) {
        let now = self.params.log_dir_timezone.now();
        self.mark_existing_logs_as_done_for_dir(&self.params.log_dir, now);
        for params in self.params.per_record.values() {
            let per_rec_now = params
                .log_dir_timezone
                .unwrap_or(self.params.log_dir_timezone)
                .now();
            if let Some(log_dir) = &params.log_dir {
                self.mark_existing_logs_as_done_for_dir(log_dir, per_rec_now);
            }
        }
    }

    fn mark_existing_logs_as_done_for_dir(
        &self,
        dir: &LogDirectory,
        now: chrono::DateTime<FixedOffset>,
    ) {
        match dir.startup_scan_dirs(now) {
            Ok(dirs) => {
                for path in dirs {
                    if let Err(err) = mark_existing_logs_as_done_in_dir(&path) {
                        tracing::error!("Error: {err:#}");
                    }
                }
            }
            Err(err) => {
                tracing::error!("Error resolving log directory {}: {err:#}", dir.display());
            }
        }
    }

    fn expire(&mut self) {
        let now = Instant::now();
        self.file_map.retain(|_, of| match of.expires {
            Some(exp) => {
                tracing::trace!("check {exp:?} vs {now:?} -> {}", exp > now);
                exp > now
            }
            None => true,
        });
    }

    fn get_deadline(&self) -> Option<Instant> {
        self.file_map.values().filter_map(|of| of.expires).min()
    }

    fn per_record(&self, kind: RecordType) -> Option<&LogRecordParams> {
        self.params
            .per_record
            .get(&kind)
            .or_else(|| self.params.per_record.get(&RecordType::Any))
    }

    fn evict_stale_dir_entries(&mut self, dir_id: usize, is_dynamic: bool, active_dir: &Path) {
        if !is_dynamic {
            return;
        }

        self.file_map.retain(|key, _| {
            if key.dir_id != dir_id {
                return true;
            }

            key.log_dir == active_dir
        });
    }

    fn resolve_template<'a>(
        params: &LogFileParams,
        template_engine: &'a TemplateEngine,
        kind: RecordType,
    ) -> Option<Template<'a, 'a>> {
        if let Some(pr) = params.per_record.get(&kind) {
            if pr.template.is_some() {
                let label = format!("{kind:?}");
                return template_engine.get_template(&label).ok();
            }
            return None;
        }
        if let Some(pr) = params.per_record.get(&RecordType::Any) {
            if pr.template.is_some() {
                return template_engine.get_template("Any").ok();
            }
        }
        None
    }

    fn do_record(&mut self, record: JsonLogRecord) -> anyhow::Result<()> {
        tracing::trace!("do_record {record:?}");
        let (dir_config, suffix, tz, segment_header) = {
            if let Some(per_rec) = self.per_record(record.kind) {
                (
                    per_rec.log_dir.as_ref().unwrap_or(&self.params.log_dir),
                    per_rec.suffix.clone(),
                    per_rec
                        .log_dir_timezone
                        .unwrap_or(self.params.log_dir_timezone),
                    if per_rec.segment_header.is_empty() {
                        None
                    } else {
                        Some(per_rec.segment_header.clone())
                    },
                )
            } else {
                (
                    &self.params.log_dir,
                    None,
                    self.params.log_dir_timezone,
                    None,
                )
            }
        };

        let now = tz.now();

        let dir_id = dir_config.id();
        let is_dynamic = dir_config.is_dynamic();
        let resolved_dir = dir_config.resolve(now)?;
        self.evict_stale_dir_entries(dir_id, is_dynamic, resolved_dir.as_path());

        let file_key = FileNameKey {
            dir_id,
            log_dir: resolved_dir.clone(),
            suffix,
        };

        // open new segment if not exists
        if !self.file_map.contains_key(&file_key) {
            let mut base_name = now.format("%Y%m%d-%H%M%S%.f").to_string();
            if let Some(suffix) = &file_key.suffix {
                base_name.push_str(suffix);
            }

            let name = resolved_dir.join(base_name);
            let log_dir = name
                .parent()
                .expect("log_dir.join ensures we always have a parent");

            let f = match std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&name)
            {
                Ok(f) => f,
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::NotFound {
                        match std::fs::create_dir_all(log_dir) {
                            Ok(_) => std::fs::OpenOptions::new()
                                .append(true)
                                .create(true)
                                .open(&name)
                                .with_context(|| format!("open log file {name:?}"))?,
                            Err(dir_err) => {
                                anyhow::bail!(
                                    "open log file {name:?}: failed: {err:#?}. \
                                    Additionally, attempting to create dir {} failed: {dir_err:#?}",
                                    log_dir.display()
                                );
                            }
                        }
                    } else {
                        anyhow::bail!("open log file {name:?}: failed: {err:#?}");
                    }
                }
            };

            let mut file = OpenedFile {
                file: Encoder::new(f, self.params.compression_level)
                    .context("set up zstd encoder")?,
                name,
                written: 0,
                expires: self
                    .params
                    .max_segment_duration
                    .map(|duration| Instant::now() + duration),
            };

            // Header di segmento (se configurato)
            if let Some(header) = &segment_header {
                file.file.write_all(header.as_bytes()).with_context(|| {
                    format!(
                        "writing segment header to newly opened segment file {}",
                        file.name.display()
                    )
                })?;
            }

            self.file_map.insert(file_key.clone(), file);
        }

        if let Some(file) = self.file_map.get_mut(&file_key) {
            let mut record_text = Vec::new();
            self.template_engine.add_global("log_record", &record)?;

            if let Some(template) =
                Self::resolve_template(&self.params, &self.template_engine, record.kind)
            {
                template
                    .render_to_write(&record, &mut record_text)
                    .context("rendering templated log record")?;
            } else {
                serde_json::to_writer(&mut record_text, &record).context("serializing record")?;
            }

            if record_text.last() != Some(&b'\n') {
                record_text.push(b'\n');
            }

            file.file
                .write_all(&record_text)
                .with_context(|| format!("writing record to {}", file.name.display()))?;
            file.written += record_text.len() as u64;

            let need_rotate = file.written >= self.params.max_file_size
                || file
                    .expires
                    .map(|exp| exp <= Instant::now())
                    .unwrap_or(false);

            if need_rotate {
                self.file_map.remove(&file_key);
            }
        }

        Ok(())
    }
}
