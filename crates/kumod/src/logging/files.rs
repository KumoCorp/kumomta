use crate::logging::{LogCommand, LogRecordParams};
use anyhow::Context;
use async_channel::Receiver;
use chrono::Utc;
pub use kumo_log_types::*;
use minijinja::{Environment, Template};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use zstd::stream::write::Encoder;

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LogFileParams {
    /// Where to place the log files
    pub log_dir: PathBuf,
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
    log_dir: PathBuf,
    suffix: Option<String>,
}

pub(crate) struct OpenedFile {
    file: Encoder<'static, File>,
    name: PathBuf,
    written: u64,
    expires: Option<Instant>,
}

impl Drop for OpenedFile {
    fn drop(&mut self) {
        self.file.do_finish().ok();
        mark_path_as_done(&self.name).ok();
        tracing::debug!("Flushed {:?}", self.name);
    }
}

fn mark_path_as_done(path: &PathBuf) -> std::io::Result<()> {
    let meta = path.metadata()?;
    // Remove the `w` bit to signal to the tailer that this
    // file will not be written to any more and that it is
    // now considered to be complete
    let mut perms = meta.permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms)
}

fn mark_existing_logs_as_done_in_dir(dir: &PathBuf) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading dir {dir:?}"))? {
        if let Ok(entry) = entry {
            match entry.file_name().to_str() {
                Some(name) if name.starts_with('.') => {
                    continue;
                }
                None => continue,
                Some(_name) => {
                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_file() {
                            mark_path_as_done(&entry.path()).ok();
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub struct LogThreadState {
    pub params: LogFileParams,
    pub receiver: Receiver<LogCommand>,
    pub template_engine: Environment<'static>,
    pub file_map: HashMap<FileNameKey, OpenedFile>,
}

impl LogThreadState {
    pub async fn logger_thread(&mut self) {
        tracing::debug!("LogFileParams: {:#?}", self.params);

        self.mark_existing_logs_as_done();
        let mut expire_counter = 0u16;

        loop {
            let deadline = self.get_deadline();
            tracing::debug!("waiting until deadline={deadline:?} for a log record");

            let cmd = if let Some(deadline) = deadline {
                tokio::select! {
                    cmd = self.receiver.recv() => {
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
                    _ = tokio::time::sleep_until(deadline.into()) => {
                        tracing::debug!("deadline reached, running expiration for this segment");
                        self.expire();
                        continue;
                    }
                }
            } else {
                self.receiver.recv().await
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
                LogCommand::Record(record) => {
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
        if let Err(err) = mark_existing_logs_as_done_in_dir(&self.params.log_dir) {
            tracing::error!("Error: {err:#}");
        }
        for params in self.params.per_record.values() {
            if let Some(log_dir) = &params.log_dir {
                if let Err(err) = mark_existing_logs_as_done_in_dir(log_dir) {
                    tracing::error!("Error: {err:#}");
                }
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
        self.file_map
            .values()
            .filter_map(|of| of.expires.clone())
            .min()
    }

    fn per_record(&self, kind: RecordType) -> Option<&LogRecordParams> {
        self.params
            .per_record
            .get(&kind)
            .or_else(|| self.params.per_record.get(&RecordType::Any))
    }

    fn resolve_template<'a>(
        params: &LogFileParams,
        template_engine: &'a Environment,
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
        let file_key = if let Some(per_rec) = self.per_record(record.kind) {
            FileNameKey {
                log_dir: per_rec
                    .log_dir
                    .as_deref()
                    .unwrap_or(&self.params.log_dir)
                    .to_path_buf(),
                suffix: per_rec.suffix.clone(),
            }
        } else {
            // Just use the global settings
            FileNameKey {
                log_dir: self.params.log_dir.clone(),
                suffix: None,
            }
        };

        if !self.file_map.contains_key(&file_key) {
            let now = Utc::now();

            let mut base_name = now.format("%Y%m%d-%H%M%S").to_string();
            if let Some(suffix) = &file_key.suffix {
                base_name.push_str(suffix);
            }

            let name = file_key.log_dir.join(base_name);

            let f = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&name)
                .with_context(|| format!("open log file {name:?}"))?;

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

            if let Some(per_rec) = self.per_record(record.kind) {
                if !per_rec.segment_header.is_empty() {
                    file.file
                        .write_all(per_rec.segment_header.as_bytes())
                        .with_context(|| {
                            format!(
                                "writing segment header to newly opened segment file {}",
                                file.name.display()
                            )
                        })?;
                }
            }

            self.file_map.insert(file_key.clone(), file);
        }

        let mut need_rotate = false;

        if let Some(file) = self.file_map.get_mut(&file_key) {
            let mut record_text = Vec::new();
            self.template_engine
                .add_global("log_record", minijinja::Value::from_serialize(&record));

            if let Some(template) =
                Self::resolve_template(&self.params, &self.template_engine, record.kind)
            {
                template.render_to_write(&record, &mut record_text)?;
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

            need_rotate = file.written >= self.params.max_file_size;
        }

        if need_rotate {
            self.file_map.remove(&file_key);
        }

        Ok(())
    }
}
