use crate::queue::QueueManager;
use crate::smtp_server::RelayDisposition;
use anyhow::{anyhow, Context};
use async_channel::{Receiver, Sender};
use bounce_classify::{BounceClass, BounceClassifier, BounceClassifierBuilder};
use chrono::Utc;
use config::{any_err, from_lua_value, get_or_create_module, load_config, CallbackSignature};
use kumo_log_types::rfc3464::ReportAction;
pub use kumo_log_types::*;
use kumo_server_runtime::rt_spawn_non_blocking;
use message::{EnvelopeAddress, Message};
use minijinja::{Environment, Template};
use minijinja_contrib::add_to_environment;
use mlua::{Lua, Value as LuaValue};
use once_cell::sync::{Lazy, OnceCell};
use rfc5321::{EnhancedStatusCode, Response, TlsInformation};
use serde::Deserialize;
use serde_json::Value;
use spool::SpoolId;
use std::collections::HashMap;
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;
use zstd::stream::write::Encoder;

static LOGGER: Lazy<Mutex<Vec<Arc<Logger>>>> = Lazy::new(|| Mutex::new(vec![]));
static CLASSIFY: OnceCell<BounceClassifier> = OnceCell::new();
pub static SHOULD_ENQ_LOG_RECORD_SIG: Lazy<CallbackSignature<(Message, String), bool>> =
    Lazy::new(|| CallbackSignature::new_with_multiple("should_enqueue_log_record"));

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

    /// Written to the start of each newly created log file segment
    #[serde(default)]
    pub segment_header: String,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LogHookParams {
    /// The unique name to identify this instance of the log hook
    pub name: String,

    /// Maximum number of outstanding items to be logged before
    /// the submission will block; helps to avoid runaway issues
    /// spiralling out of control.
    #[serde(default = "LogFileParams::default_back_pressure")]
    pub back_pressure: usize,

    /// List of meta fields to capture in the log
    #[serde(default)]
    pub meta: Vec<String>,

    /// List of message headers to capture in the log
    #[serde(default)]
    pub headers: Vec<String>,

    #[serde(default)]
    pub per_record: HashMap<RecordType, LogRecordParams>,

    #[serde(default)]
    pub deferred_spool: bool,
}

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

    /// The name of an event which can be used to filter
    /// out log records which should not be logged to this
    /// log file
    #[serde(default)]
    pub filter_event: Option<String>,
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
    thread: TokioMutex<Option<JoinHandle<()>>>,
    meta: Vec<String>,
    headers: Vec<String>,
    enabled: HashMap<RecordType, bool>,
    filter_event: Option<String>,
}

impl Logger {
    fn get_loggers() -> Vec<Arc<Logger>> {
        LOGGER.lock().unwrap().iter().map(Arc::clone).collect()
    }

    pub fn init_hook(params: LogHookParams) -> anyhow::Result<()> {
        let mut template_engine = Environment::new();
        add_to_environment(&mut template_engine);

        for (kind, per_rec) in &params.per_record {
            if let Some(template_source) = &per_rec.template {
                template_engine
                    .add_template_owned(format!("{kind:?}"), template_source.clone())
                    .with_context(|| {
                        format!(
                            "compiling template:\n{template_source}\nfor log record type {kind:?}"
                        )
                    })?;
            }
        }

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
                    let mut state = LogHookState {
                        params,
                        receiver,
                        template_engine,
                    };
                    state.logger_thread().await
                });
            })?;

        let logger = Self {
            sender,
            thread: TokioMutex::new(Some(thread)),
            meta,
            headers,
            enabled,
            filter_event: None,
        };

        LOGGER.lock().unwrap().push(Arc::new(logger));
        Ok(())
    }

    pub fn init(params: LogFileParams) -> anyhow::Result<()> {
        let mut template_engine = Environment::new();
        add_to_environment(&mut template_engine);

        for (kind, per_rec) in &params.per_record {
            if let Some(template_source) = &per_rec.template {
                template_engine
                    .add_template_owned(format!("{kind:?}"), template_source.clone())
                    .with_context(|| {
                        format!(
                            "compiling template:\n{template_source}\nfor log record type {kind:?}"
                        )
                    })?;
            }
        }

        std::fs::create_dir_all(&params.log_dir)
            .with_context(|| format!("creating log directory {}", params.log_dir.display()))?;

        let mut enabled = HashMap::new();
        for (kind, cfg) in &params.per_record {
            enabled.insert(*kind, cfg.enable);
        }

        let headers = params.headers.clone();
        let meta = params.meta.clone();
        let (sender, receiver) = async_channel::bounded(params.back_pressure);
        let filter_event = params.filter_event.clone();

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
            thread: TokioMutex::new(Some(thread)),
            meta,
            headers,
            enabled,
            filter_event,
        };

        LOGGER.lock().unwrap().push(Arc::new(logger));
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

    pub fn signal_shutdown() -> Pin<Box<dyn Future<Output = ()>>> {
        Box::pin(async move {
            let loggers = Self::get_loggers();
            for logger in loggers.iter() {
                tracing::debug!("Terminating a logger");
                logger.sender.send(LogCommand::Terminate).await.ok();
                tracing::debug!("Joining that logger");
                let res = logger
                    .thread
                    .lock()
                    .await
                    .take()
                    .map(|thread| thread.join());
                tracing::debug!("Joined -> {res:?}");
            }
        })
    }

    pub async fn extract_fields(
        &self,
        msg: &Message,
    ) -> (HashMap<String, Value>, HashMap<String, Value>) {
        let mut headers = HashMap::new();
        let mut meta = HashMap::new();

        if !self.headers.is_empty() {
            msg.load_data_if_needed().await.ok();

            let mut all_headers: HashMap<String, (String, Vec<Value>)> = HashMap::new();
            for (name, value) in msg.get_all_headers().unwrap_or_else(|_| vec![]) {
                all_headers
                    .entry(name.to_ascii_lowercase())
                    .or_insert_with(|| (name.to_string(), vec![]))
                    .1
                    .push(value.into());
            }

            fn capture_header(
                headers: &mut HashMap<String, Value>,
                name: &str,
                all_headers: &mut HashMap<String, (String, Vec<Value>)>,
            ) {
                match all_headers.remove(&name.to_ascii_lowercase()) {
                    Some((orig_name, mut values)) if values.len() == 1 => {
                        headers.insert(orig_name.to_string(), values.remove(0));
                    }
                    Some((orig_name, values)) => {
                        headers.insert(orig_name.to_string(), Value::Array(values));
                    }
                    None => {}
                }
            }

            for name in &self.headers {
                if name.ends_with('*') {
                    let pattern = name[..name.len() - 1].to_ascii_lowercase();
                    let matching_names: Vec<String> = all_headers
                        .keys()
                        .filter_map(|candidate| {
                            if candidate.to_ascii_lowercase().starts_with(&pattern) {
                                Some(candidate.to_string())
                            } else {
                                None
                            }
                        })
                        .collect();
                    for name in matching_names {
                        capture_header(&mut headers, &name, &mut all_headers);
                    }
                } else {
                    capture_header(&mut headers, name, &mut all_headers);
                }
            }
        }

        for name in &self.meta {
            if let Ok(value) = msg.get_meta(name) {
                if !value.is_null() {
                    meta.insert(name.to_string(), value);
                }
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
    pub delivery_protocol: Option<&'a str>,
    pub tls_info: Option<&'a TlsInformation>,
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
        delivery_protocol,
        tls_info,
    } = args;

    let loggers = Logger::get_loggers();
    if loggers.is_empty() {
        return;
    }

    let mut feedback_report = None;

    msg.load_meta_if_needed().await.ok();

    let reception_protocol = msg.get_meta_string("reception_protocol").unwrap_or(None);

    if kind == RecordType::Reception {
        if let Some(RelayDisposition { log_arf: true, .. }) = relay_disposition {
            if let Ok(Some(report)) = msg.parse_rfc5965() {
                feedback_report.replace(report);
                kind = RecordType::Feedback;
            }
        }
    }

    let now = Utc::now();
    let nodeid = kumo_server_common::nodeid::NodeId::get_uuid();

    for logger in loggers.iter() {
        if !logger.record_is_enabled(kind) {
            continue;
        }
        if let Some(name) = &logger.filter_event {
            match load_config().await {
                Ok(mut lua_config) => {
                    let log_sig = CallbackSignature::<Message, bool>::new(name.clone());

                    let enqueue: bool =
                        match lua_config.async_call_callback(&log_sig, msg.clone()).await {
                            Ok(b) => b,
                            Err(err) => {
                                tracing::error!(
                                    "error while calling {name} event for log filter: {err:#}"
                                );
                                false
                            }
                        };
                    if !enqueue {
                        continue;
                    }
                }
                Err(err) => {
                    tracing::error!(
                        "failed to load lua config while attempting to \
                         call {name} event for log filter: {err:#}"
                    );
                    continue;
                }
            };
        }

        match kind {
            RecordType::Reception => {
                crate::accounting::account_reception(
                    &reception_protocol.as_deref().unwrap_or("unknown"),
                );
            }
            RecordType::Delivery => {
                crate::accounting::account_delivery(
                    &delivery_protocol.as_deref().unwrap_or("unknown"),
                );
            }
            _ => {}
        };

        let (headers, meta) = logger.extract_fields(&msg).await;

        let mut tls_cipher = None;
        let mut tls_protocol_version = None;
        let mut tls_peer_subject_name = None;
        if let Some(info) = tls_info {
            tls_cipher.replace(info.cipher.clone());
            tls_protocol_version.replace(info.protocol_version.clone());
            tls_peer_subject_name.replace(info.subject_name.clone());
        }

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
            response: response.clone(),
            timestamp: now,
            created: msg.id().created(),
            num_attempts: msg.get_num_attempts(),
            egress_pool: egress_pool.map(|s| s.to_string()),
            egress_source: egress_source.map(|s| s.to_string()),
            bounce_classification: BounceClass::default(),
            feedback_report: feedback_report.clone(),
            headers,
            meta,
            delivery_protocol: delivery_protocol.map(|s| s.to_string()),
            reception_protocol: reception_protocol.clone(),
            nodeid,
            tls_cipher,
            tls_protocol_version,
            tls_peer_subject_name,
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
                            bounce_classification: BounceClass::default(),
                            feedback_report: None,
                            headers: HashMap::new(),
                            meta: HashMap::new(),
                            delivery_protocol: None,
                            reception_protocol: reception_protocol.clone(),
                            nodeid,
                            tls_cipher: None,
                            tls_protocol_version: None,
                            tls_peer_subject_name: None,
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

struct LogHookState {
    params: LogHookParams,
    receiver: Receiver<LogCommand>,
    template_engine: Environment<'static>,
}

impl LogHookState {
    async fn logger_thread(&mut self) {
        tracing::debug!("LogHookParams: {:#?}", self.params);

        loop {
            let cmd = match self.receiver.recv().await {
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

    fn do_record(&mut self, mut record: JsonLogRecord) -> anyhow::Result<()> {
        tracing::trace!("do_record {record:?}");

        if let Some(classifier) = CLASSIFY.get() {
            record.bounce_classification = classifier.classify_response(&record.response);
        }

        let mut record_text = Vec::new();
        self.template_engine
            .add_global("log_record", minijinja::Value::from_serializable(&record));

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

        let record_json = serde_json::to_value(&record)?;

        let id = SpoolId::new();
        let msg = Message::new_dirty(
            id,
            EnvelopeAddress::parse(&record.sender)?,
            EnvelopeAddress::parse(&record.recipient)?,
            record.meta.clone().into_iter().collect(),
            Arc::new(record_text.into_boxed_slice()),
        )?;

        msg.set_meta("log_record", record_json)?;
        msg.set_meta("reception_protocol", "LogRecord")?;
        let deferred_spool = self.params.deferred_spool;
        let name = self.params.name.clone();

        rt_spawn_non_blocking("should_enqueue_log_record".to_string(), move || {
            Ok(async move {
                let mut lua_config = load_config().await?;

                let enqueue: bool = lua_config
                    .async_call_callback(&SHOULD_ENQ_LOG_RECORD_SIG, (msg.clone(), name))
                    .await?;

                if enqueue {
                    let queue_name = msg.get_queue_name()?;
                    if !deferred_spool {
                        msg.save().await?;
                    }
                    QueueManager::insert(&queue_name, msg).await?;
                }

                anyhow::Result::<()>::Ok(())
            })
        })?;

        Ok(())
    }

    fn resolve_template<'a>(
        params: &LogHookParams,
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
            tracing::debug!("waiting until deadline={deadline:?} for a log record");

            let cmd = if let Some(deadline) = deadline {
                tokio::select! {
                    cmd = self.receiver.recv() => cmd,
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
                .add_global("log_record", minijinja::Value::from_serializable(&record));

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

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "configure_bounce_classifier",
        lua.create_function(move |lua, params: LuaValue| {
            let params: ClassifierParams = from_lua_value(lua, params)?;
            params.register().map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_local_logs",
        lua.create_function(move |lua, params: LuaValue| {
            let params: LogFileParams = from_lua_value(lua, params)?;
            Logger::init(params).map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_log_hook",
        lua.create_function(move |lua, params: LuaValue| {
            let params: LogHookParams = from_lua_value(lua, params)?;
            Logger::init_hook(params).map_err(any_err)
        })?,
    )?;

    Ok(())
}
