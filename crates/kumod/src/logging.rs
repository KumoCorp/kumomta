use crate::smtp_server::RelayDisposition;
use anyhow::{anyhow, Context};
use async_channel::{Receiver, Sender};
use bounce_classify::{BounceClass, BounceClassifier, BounceClassifierBuilder};
use chrono::Utc;
use kumo_log_types::rfc3464::ReportAction;
pub use kumo_log_types::*;
use message::Message;
use minijinja::{Environment, Source, Template};
use once_cell::sync::OnceCell;
use rfc5321::{EnhancedStatusCode, Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use zstd::stream::write::Encoder;

static LOGGER: OnceCell<Logger> = OnceCell::new();
static CLASSIFY: OnceCell<BounceClassifier> = OnceCell::new();

#[derive(Deserialize, Clone, Debug)]
pub struct ClassifierParams {
    pub files: Vec<String>,
}

impl ClassifierParams {
    pub fn register(&self) -> anyhow::Result<()> {
        let mut builder = BounceClassifierBuilder::new();
        for file_name in &self.files {
            if file_name.ends_with(".json") {
                builder
                    .merge_json_file(file_name)
                    .map_err(|err| anyhow!("{err}"))?;
            } else if file_name.ends_with(".toml") {
                builder
                    .merge_toml_file(file_name)
                    .map_err(|err| anyhow!("{err}"))?;
            } else {
                anyhow::bail!("{file_name}: classifier files must have either .toml or .json filename extension");
            }
        }

        let classifier = builder.build().map_err(|err| anyhow!("{err}"))?;

        CLASSIFY
            .set(classifier)
            .map_err(|_| anyhow::anyhow!("classifieer already initialized"))?;

        Ok(())
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct LogRecordParams {
    #[serde(default)]
    pub suffix: Option<String>,

    /// Where to place the log files; overrides the global setting
    #[serde(default)]
    pub log_dir: Option<PathBuf>,

    #[serde(default = "default_true")]
    pub enable: bool,

    /// Instead of logging the json object, format it with this
    /// minijinja template
    #[serde(default)]
    pub template: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
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

    #[serde(default, with = "humantime_serde")]
    pub max_segment_duration: Option<Duration>,

    /// List of meta fields to capture in the log
    #[serde(default)]
    pub meta: Vec<String>,

    /// List of message headers to capture in the log
    #[serde(default)]
    pub headers: Vec<String>,

    #[serde(default)]
    pub per_record: HashMap<RecordType, LogRecordParams>,
}

impl LogFileParams {
    fn default_max_file_size() -> u64 {
        1_000_000_000
    }
    fn default_back_pressure() -> usize {
        128_000
    }
    fn default_compression_level() -> i32 {
        0 // use the zstd default
    }
}

#[derive(Debug)]
enum LogCommand {
    Record(JsonLogRecord),
    Terminate,
}

pub struct Logger {
    sender: Sender<LogCommand>,
    thread: Mutex<Option<JoinHandle<()>>>,
    meta: Vec<String>,
    headers: Vec<String>,
    enabled: HashMap<RecordType, bool>,
}

impl Logger {
    pub fn get() -> Option<&'static Logger> {
        LOGGER.get()
    }

    pub fn init(params: LogFileParams) -> anyhow::Result<()> {
        let mut source = Source::new();

        for (kind, per_rec) in &params.per_record {
            if let Some(template_source) = &per_rec.template {
                source
                    .add_template(format!("{kind:?}"), template_source)
                    .with_context(|| {
                        format!(
                            "compiling template:\n{template_source}\nfor log record type {kind:?}"
                        )
                    })?;
            }
        }

        let mut template_engine = Environment::new();
        template_engine.set_source(source);

        std::fs::create_dir_all(&params.log_dir)
            .with_context(|| format!("creating log directory {}", params.log_dir.display()))?;

        let mut enabled = HashMap::new();
        for (kind, cfg) in &params.per_record {
            enabled.insert(*kind, cfg.enable);
        }

        let headers = params.headers.clone();
        let meta = params.meta.clone();
        let (sender, receiver) = async_channel::bounded(params.back_pressure);
        let thread = std::thread::Builder::new()
            .name("logger".to_string())
            .spawn(move || {
                tracing::debug!("started logger thread");
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .expect("create logger runtime");
                runtime.block_on(async move {
                    tracing::debug!("calling state.logger_thread()");
                    let mut state = LogThreadState {
                        params,
                        receiver,
                        template_engine,
                        file_map: HashMap::new(),
                    };
                    state.logger_thread().await
                });
            })?;

        let logger = Self {
            sender,
            thread: Mutex::new(Some(thread)),
            meta,
            headers,
            enabled,
        };

        LOGGER
            .set(logger)
            .map_err(|_| anyhow::anyhow!("logger already initialized"))?;
        Ok(())
    }

    pub fn record_is_enabled(&self, kind: RecordType) -> bool {
        if let Some(enabled) = self.enabled.get(&kind) {
            return *enabled;
        }
        if let Some(enabled) = self.enabled.get(&RecordType::Any) {
            return *enabled;
        }
        true
    }

    pub async fn log(&self, record: JsonLogRecord) -> anyhow::Result<()> {
        Ok(self.sender.send(LogCommand::Record(record)).await?)
    }

    pub async fn signal_shutdown() {
        if let Some(logger) = Self::get() {
            logger.sender.send(LogCommand::Terminate).await.ok();
            logger
                .thread
                .lock()
                .unwrap()
                .take()
                .map(|thread| thread.join());
        }
    }

    pub async fn extract_fields(
        &self,
        msg: &Message,
    ) -> (HashMap<String, Value>, HashMap<String, Value>) {
        let mut headers = HashMap::new();
        let mut meta = HashMap::new();

        if !self.headers.is_empty() {
            msg.load_data_if_needed().await.ok();

            let mut all_headers: HashMap<String, Vec<Value>> = HashMap::new();
            for (name, value) in msg.get_all_headers().unwrap_or_else(|_| vec![]) {
                all_headers
                    .entry(name.to_ascii_lowercase())
                    .or_default()
                    .push(value.into());
            }

            for name in &self.headers {
                match all_headers.remove(&name.to_ascii_lowercase()) {
                    Some(mut values) if values.len() == 1 => {
                        headers.insert(name.to_string(), values.remove(0));
                    }
                    Some(values) => {
                        headers.insert(name.to_string(), Value::Array(values));
                    }
                    None => {}
                }
            }
        }

        for name in &self.meta {
            if let Ok(value) = msg.get_meta(name) {
                meta.insert(name.to_string(), value);
            }
        }

        (headers, meta)
    }
}

pub struct LogDisposition<'a> {
    pub kind: RecordType,
    pub msg: Message,
    pub site: &'a str,
    pub peer_address: Option<&'a ResolvedAddress>,
    pub response: Response,
    pub egress_pool: Option<&'a str>,
    pub egress_source: Option<&'a str>,
    pub relay_disposition: Option<RelayDisposition>,
}

pub async fn log_disposition(args: LogDisposition<'_>) {
    let LogDisposition {
        mut kind,
        msg,
        site,
        peer_address,
        response,
        egress_pool,
        egress_source,
        relay_disposition,
    } = args;

    if let Some(logger) = Logger::get() {
        let mut feedback_report = None;

        msg.load_meta_if_needed().await.ok();

        if kind == RecordType::Reception {
            if let Some(RelayDisposition { log_arf: true, .. }) = relay_disposition {
                if let Ok(Some(report)) = msg.parse_rfc5965() {
                    feedback_report.replace(report);
                    kind = RecordType::Feedback;
                }
            }
        }

        if !logger.record_is_enabled(kind) {
            return;
        }

        let (headers, meta) = logger.extract_fields(&msg).await;

        let record = JsonLogRecord {
            kind,
            id: msg.id().to_string(),
            size: msg.get_data().len() as u64,
            sender: msg
                .sender()
                .map(|addr| addr.to_string())
                .unwrap_or_else(|err| format!("{err:#}")),
            recipient: msg
                .recipient()
                .map(|addr| addr.to_string())
                .unwrap_or_else(|err| format!("{err:#}")),
            queue: msg
                .get_queue_name()
                .unwrap_or_else(|err| format!("{err:#}")),
            site: site.to_string(),
            peer_address: peer_address.cloned(),
            response,
            timestamp: Utc::now(),
            created: msg.id().created(),
            num_attempts: msg.get_num_attempts(),
            egress_pool: egress_pool.map(|s| s.to_string()),
            egress_source: egress_source.map(|s| s.to_string()),
            bounce_classification: BounceClass::Uncategorized,
            feedback_report,
            headers,
            meta,
        };
        if let Err(err) = logger.log(record).await {
            tracing::error!("failed to log: {err:#}");
        }

        if kind == RecordType::Reception {
            if let Some(RelayDisposition { log_oob: true, .. }) = relay_disposition {
                if let Ok(Some(report)) = msg.parse_rfc3464() {
                    // This incoming bounce report is addressed to
                    // the envelope from of the original message
                    let sender = msg
                        .recipient()
                        .map(|addr| addr.to_string())
                        .unwrap_or_else(|err| format!("{err:#}"));
                    let queue = msg
                        .get_queue_name()
                        .unwrap_or_else(|err| format!("{err:#}"));

                    for recip in &report.per_recipient {
                        if recip.action != ReportAction::Failed {
                            continue;
                        }

                        let enhanced_code = EnhancedStatusCode {
                            class: recip.status.class,
                            subject: recip.status.subject,
                            detail: recip.status.detail,
                        };

                        let (code, content) = match &recip.diagnostic_code {
                            Some(diag) if diag.diagnostic_type == "smtp" => {
                                if let Some((code, content)) = diag.diagnostic.split_once(' ') {
                                    if let Ok(code) = code.parse() {
                                        (code, content.to_string())
                                    } else {
                                        (550, diag.diagnostic.to_string())
                                    }
                                } else {
                                    (550, diag.diagnostic.to_string())
                                }
                            }
                            _ => (550, "".to_string()),
                        };

                        let record = JsonLogRecord {
                            kind: RecordType::OOB,
                            id: msg.id().to_string(),
                            size: 0,
                            sender: sender.clone(),
                            recipient: recip
                                .original_recipient
                                .as_ref()
                                .unwrap_or(&recip.final_recipient)
                                .recipient
                                .to_string(),
                            queue: queue.to_string(),
                            site: site.to_string(),
                            peer_address: Some(ResolvedAddress {
                                name: report.per_message.reporting_mta.name.to_string(),
                                addr: peer_address
                                    .map(|a| a.addr)
                                    .unwrap_or_else(|| Ipv4Addr::UNSPECIFIED.into()),
                            }),
                            response: Response {
                                code,
                                enhanced_code: Some(enhanced_code),
                                content,
                                command: None,
                            },
                            timestamp: recip.last_attempt_date.unwrap_or_else(|| Utc::now()),
                            created: msg.id().created(),
                            num_attempts: 0,
                            egress_pool: None,
                            egress_source: None,
                            bounce_classification: BounceClass::Uncategorized,
                            feedback_report: None,
                            headers: HashMap::new(),
                            meta: HashMap::new(),
                        };

                        if let Err(err) = logger.log(record).await {
                            tracing::error!("failed to log: {err:#}");
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct FileNameKey {
    log_dir: PathBuf,
    suffix: Option<String>,
}

struct OpenedFile {
    file: Encoder<'static, File>,
    name: PathBuf,
    written: u64,
    expires: Option<Instant>,
}

impl Drop for OpenedFile {
    fn drop(&mut self) {
        self.file.do_finish().ok();
        mark_path_as_done(&self.name).ok();
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

struct LogThreadState {
    params: LogFileParams,
    receiver: Receiver<LogCommand>,
    template_engine: Environment<'static>,
    file_map: HashMap<FileNameKey, OpenedFile>,
}

impl LogThreadState {
    async fn logger_thread(&mut self) {
        tracing::debug!("LogFileParams: {:#?}", self.params);

        self.mark_existing_logs_as_done();

        loop {
            let deadline = self.get_deadline();
            tracing::debug!("waiting until {deadline:?} for log");

            let cmd = if let Some(deadline) = deadline {
                tokio::select! {
                    cmd = self.receiver.recv() => cmd,
                    _ = tokio::time::sleep_until(deadline.into()) => {
                        tracing::debug!("deadline reached, running expiration");
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
    ) -> Option<Template<'a>> {
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

    fn do_record(&mut self, mut record: JsonLogRecord) -> anyhow::Result<()> {
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

        if let Some(classifier) = CLASSIFY.get() {
            record.bounce_classification = classifier.classify_response(&record.response);
        }

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

            self.file_map.insert(
                file_key.clone(),
                OpenedFile {
                    file: Encoder::new(f, self.params.compression_level)
                        .context("set up zstd encoder")?,
                    name,
                    written: 0,
                    expires: self
                        .params
                        .max_segment_duration
                        .map(|duration| Instant::now() + duration),
                },
            );
        }

        let mut need_rotate = false;

        if let Some(file) = self.file_map.get_mut(&file_key) {
            let mut record_text = Vec::new();

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
