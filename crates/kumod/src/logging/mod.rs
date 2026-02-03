use crate::logging::classify::{apply_classification, ClassifierParams};
use crate::logging::disposition_hooks::{DispHookParams, RecordWrapper};
use crate::logging::files::{LogFileParams, LogThreadState};
use crate::logging::hooks::{LogHookParams, LogHookState};
use anyhow::Context;
use chrono::format::{Item, StrftimeItems};
use chrono::{DateTime, FixedOffset, Local, Utc};
use config::{any_err, from_lua_value, get_or_create_module, CallbackSignature};
use flume::{bounded, Sender, TrySendError};
pub use kumo_log_types::*;
use kumo_server_common::disk_space::MonitoredPath;
use kumo_server_runtime::Runtime;
use kumo_template::TemplateEngine;
use message::Message;
use mlua::{Lua, Value as LuaValue};
use parking_lot::FairMutex as Mutex;
use prometheus::{CounterVec, Histogram, HistogramVec};
use serde::de::Deserializer;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinHandle;

pub(crate) mod classify;
pub(crate) mod disposition;
pub(crate) mod disposition_hooks;
pub(crate) mod files;
pub(crate) mod hooks;
pub(crate) mod rejection;

static SUBMIT_FULL: LazyLock<CounterVec> = LazyLock::new(|| {
    prometheus::register_counter_vec!(
        "log_submit_full",
        "how many times submission of a log event hit the back_pressure",
        &["logger"]
    )
    .unwrap()
});
static SUBMIT_LATENCY: LazyLock<HistogramVec> = LazyLock::new(|| {
    prometheus::register_histogram_vec!(
        "log_submit_latency",
        "latency of log event submission operations",
        &["logger"]
    )
    .unwrap()
});
static LOGGER: LazyLock<Mutex<Vec<Arc<Logger>>>> = LazyLock::new(Mutex::default);

static LOGGING_THREADS: AtomicUsize = AtomicUsize::new(0);
static NEXT_LOG_DIR_ID: AtomicUsize = AtomicUsize::new(1);
pub static LOGGING_RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("logging", |cpus| cpus / 4, &LOGGING_THREADS).unwrap());

pub fn set_logging_threads(n: usize) {
    LOGGING_THREADS.store(n, Ordering::SeqCst);
}

#[derive(Clone, Debug)]
struct StrftimePattern {
    items: Vec<Item<'static>>,
    has_directives: bool,
}

impl StrftimePattern {
    fn new(pattern: &str) -> anyhow::Result<Self> {
        let mut items = Vec::new();
        let mut has_directives = false;

        for item in StrftimeItems::new(pattern) {
            match item {
                Item::Error => {
                    anyhow::bail!("invalid strftime directive in pattern: {pattern}");
                }
                Item::Literal(_) | Item::Space(_) => {
                    items.push(item.to_owned());
                }
                other => {
                    has_directives = true;
                    items.push(other.to_owned());
                }
            }
        }

        Ok(Self {
            items,
            has_directives,
        })
    }

    fn is_dynamic(&self) -> bool {
        self.has_directives
    }

    fn render(&self, datetime: DateTime<FixedOffset>) -> PathBuf {
        let formatted = datetime
            .format_with_items(self.items.iter().cloned())
            .to_string();
        PathBuf::from(formatted)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogDirTimeZone {
    Local,
    Utc,
}

impl LogDirTimeZone {
    fn now(&self) -> DateTime<FixedOffset> {
        match self {
            LogDirTimeZone::Local => {
                let now = Local::now();
                now.with_timezone(now.offset())
            }
            LogDirTimeZone::Utc => Utc::now().fixed_offset(),
        }
    }
}

impl Default for LogDirTimeZone {
    fn default() -> Self {
        LogDirTimeZone::Utc
    }
}

impl<'de> Deserialize<'de> for LogDirTimeZone {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: String = String::deserialize(deserializer)?;
        match value.to_ascii_lowercase().as_str() {
            "local" => Ok(LogDirTimeZone::Local),
            "utc" => Ok(LogDirTimeZone::Utc),
            other => Err(serde::de::Error::custom(format!(
                "invalid log_dir_timezone `{other}`; expected `Local` or `UTC`"
            ))),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogDirectory {
    raw: PathBuf,
    pattern: Option<StrftimePattern>,
    base_dir: Option<PathBuf>,
    id: usize,
}

impl LogDirectory {
    pub fn new(raw: PathBuf) -> anyhow::Result<Self> {
        let raw_str = raw.to_string_lossy();
        let compiled = StrftimePattern::new(raw_str.as_ref())?;
        let (pattern, base_dir) = if compiled.is_dynamic() {
            (Some(compiled), Self::compute_base_dir(&raw)?)
        } else {
            (None, Some(raw.clone()))
        };
        let id = NEXT_LOG_DIR_ID.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            raw,
            pattern,
            base_dir,
            id,
        })
    }

    pub fn display(&self) -> std::path::Display<'_> {
        self.raw.display()
    }

    pub fn is_dynamic(&self) -> bool {
        self.pattern.is_some()
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn resolve(&self, now: DateTime<FixedOffset>) -> anyhow::Result<PathBuf> {
        if let Some(pattern) = &self.pattern {
            Ok(pattern.render(now))
        } else {
            Ok(self.raw.clone())
        }
    }

    pub fn ensure_base_exists(&self, now: DateTime<FixedOffset>) -> anyhow::Result<()> {
        if let Some(base) = &self.base_dir {
            if !base.as_os_str().is_empty() {
                std::fs::create_dir_all(base)?;
            }
            return Ok(());
        }

        if !self.is_dynamic() {
            std::fs::create_dir_all(&self.raw)?;
            return Ok(());
        }

        if let Some(parent) = self.resolve(now)?.parent().map(Path::to_path_buf) {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(&parent)?;
            }
        }

        Ok(())
    }

    pub fn monitoring_path(&self, now: DateTime<FixedOffset>) -> anyhow::Result<PathBuf> {
        if let Some(base) = &self.base_dir {
            Ok(base.clone())
        } else {
            self.resolve(now)
        }
    }

    pub fn startup_scan_dirs(&self, now: DateTime<FixedOffset>) -> anyhow::Result<Vec<PathBuf>> {
        let mut dirs = Vec::new();
        if let Some(base) = &self.base_dir {
            if !base.as_os_str().is_empty() {
                dirs.push(base.clone());
            }
        }
        let resolved = self.resolve(now)?;
        if !dirs.iter().any(|dir| dir == &resolved) {
            dirs.push(resolved);
        }
        Ok(dirs)
    }

    fn compute_base_dir(raw: &Path) -> anyhow::Result<Option<PathBuf>> {
        let mut base = PathBuf::new();

        for comp in raw.components() {
            match comp {
                Component::Prefix(prefix) => {
                    base.push(prefix.as_os_str());
                }
                Component::RootDir => {
                    base.push(Path::new(std::path::MAIN_SEPARATOR_STR));
                }
                Component::CurDir => {
                    base.push(Path::new("."));
                }
                Component::ParentDir => {
                    base.push(Path::new(".."));
                }
                Component::Normal(os) => {
                    let comp_str = os.to_string_lossy();
                    let compiled = StrftimePattern::new(comp_str.as_ref())?;
                    if compiled.is_dynamic() {
                        break;
                    }
                    base.push(os);
                }
            }
        }

        if base.as_os_str().is_empty() {
            Ok(Some(base))
        } else {
            Ok(None)
        }
    }
}

impl<'de> Deserialize<'de> for LogDirectory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = PathBuf::deserialize(deserializer)?;
        LogDirectory::new(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LogRecordParams {
    #[serde(default)]
    pub suffix: Option<String>,

    /// Where to place the log files; overrides the global setting
    #[serde(default)]
    pub log_dir: Option<LogDirectory>,

    /// Override the timezone used when expanding `log_dir`
    #[serde(default)]
    pub log_dir_timezone: Option<LogDirTimeZone>,

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

#[derive(Debug)]
pub(crate) enum LogCommand {
    Record(JsonLogRecord, Option<Message>),
    Terminate,
}

enum LoggerImpl {
    /// Queued to another thread to dispatch
    Queue {
        sender: Sender<LogCommand>,
        thread: TokioMutex<Option<JoinHandle<()>>>,
    },
    /// Processed immediate in the context of log_disposition.
    /// Necessary for things that might operate on (eg: read) the
    /// originating message prior to it being removed from the spool.
    /// We can't queue those operations because the caller might remove
    /// the message from the spool before the log event would be
    /// dispatched.
    Immediate(Arc<CallbackSignature<(Message, RecordWrapper), ()>>),
}

pub struct Logger {
    implementation: LoggerImpl,
    meta: Vec<String>,
    headers: Vec<String>,
    enabled: HashMap<RecordType, bool>,
    filter_event: Option<String>,
    hook_name: Option<String>,
    name: String,
    submit_latency: Histogram,
}

impl Logger {
    fn get_loggers() -> Vec<Arc<Logger>> {
        LOGGER.lock().iter().map(Arc::clone).collect()
    }

    pub async fn init_disp_hook(params: DispHookParams) -> anyhow::Result<()> {
        let mut loggers = LOGGER.lock();

        if loggers
            .iter()
            .any(|existing| existing.hook_name.as_deref() == Some(params.name.as_str()))
        {
            anyhow::bail!(
                "A logging hook with name `{}` has already been registered",
                params.name
            );
        }

        let mut enabled = HashMap::new();
        for (kind, cfg) in &params.per_record {
            enabled.insert(*kind, cfg.enable);
        }

        let hook_name = params.name.to_string();
        let name = format!("hook-{hook_name}");

        let sig = CallbackSignature::new(format!("log_disposition_{hook_name}"));

        let submit_latency = SUBMIT_LATENCY.get_metric_with_label_values(&[&name])?;

        let logger = Self {
            implementation: LoggerImpl::Immediate(Arc::new(sig)),
            meta: Default::default(),
            headers: Default::default(),
            enabled,
            filter_event: None,
            hook_name: Some(hook_name),
            name,
            submit_latency,
        };

        loggers.push(Arc::new(logger));
        Ok(())
    }

    pub async fn init_hook(params: LogHookParams) -> anyhow::Result<()> {
        let mut loggers = LOGGER.lock();

        if loggers
            .iter()
            .any(|existing| existing.hook_name.as_deref() == Some(params.name.as_str()))
        {
            anyhow::bail!(
                "A logging hook with name `{}` has already been registered",
                params.name
            );
        }

        let mut template_engine = TemplateEngine::new();

        for (kind, per_rec) in &params.per_record {
            if let Some(template_source) = &per_rec.template {
                template_engine
                    .add_template(format!("{kind:?}"), template_source.clone())
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
        let hook_name = params.name.to_string();
        let name = format!("hook-{hook_name}");
        let (sender, receiver) = bounded(params.back_pressure);

        let mut state = LogHookState::new(params, receiver, template_engine);

        let thread = LOGGING_RUNTIME.spawn("log hook".to_string(), async move {
            tracing::debug!("calling state.logger_thread()");
            state.logger_thread().await
        })?;

        let submit_latency = SUBMIT_LATENCY.get_metric_with_label_values(&[&name])?;
        let implementation = LoggerImpl::Queue {
            sender,
            thread: TokioMutex::new(Some(thread)),
        };

        let logger = Self {
            implementation,
            meta,
            headers,
            enabled,
            filter_event: None,
            hook_name: Some(hook_name),
            name,
            submit_latency,
        };

        loggers.push(Arc::new(logger));
        Ok(())
    }

    pub async fn init(params: LogFileParams) -> anyhow::Result<()> {
        let mut template_engine = TemplateEngine::new();

        for (kind, per_rec) in &params.per_record {
            if let Some(template_source) = &per_rec.template {
                template_engine
                    .add_template(format!("{kind:?}"), template_source.clone())
                    .with_context(|| {
                        format!(
                            "compiling template:\n{template_source}\nfor log record type {kind:?}"
                        )
                    })?;
            }
        }

        let global_now = params.log_dir_timezone.now();

        params
            .log_dir
            .ensure_base_exists(global_now)
            .with_context(|| format!("creating log directory {}", params.log_dir.display()))?;
        for per_rec in params.per_record.values() {
            if let Some(dir) = &per_rec.log_dir {
                let tz = per_rec
                    .log_dir_timezone
                    .unwrap_or(params.log_dir_timezone)
                    .now();
                dir.ensure_base_exists(tz)
                    .with_context(|| format!("creating log directory {}", dir.display()))?;
            }
        }

        let monitored_path = params
            .log_dir
            .monitoring_path(global_now)
            .with_context(|| {
                format!("resolving monitoring path for {}", params.log_dir.display())
            })?;

        let mut enabled = HashMap::new();
        for (kind, cfg) in &params.per_record {
            enabled.insert(*kind, cfg.enable);
        }

        let headers = params.headers.clone();
        let meta = params.meta.clone();
        let (sender, receiver) = bounded(params.back_pressure);
        let filter_event = params.filter_event.clone();
        let name = format!("dir-{}", params.log_dir.display());

        MonitoredPath {
            name: format!("log dir {}", params.log_dir.display()),
            path: monitored_path,
            min_free_space: params.min_free_space,
            min_free_inodes: params.min_free_inodes,
        }
        .register();

        let thread = LOGGING_RUNTIME.spawn("log file".to_string(), async move {
            tracing::debug!("calling state.logger_thread()");
            let mut state = LogThreadState {
                params,
                receiver,
                template_engine,
                file_map: HashMap::new(),
            };
            state.logger_thread().await
        })?;

        let submit_latency = SUBMIT_LATENCY.get_metric_with_label_values(&[&name])?;

        let implementation = LoggerImpl::Queue {
            sender,
            thread: TokioMutex::new(Some(thread)),
        };

        let logger = Self {
            implementation,
            meta,
            headers,
            enabled,
            filter_event,
            hook_name: None,
            name,
            submit_latency,
        };

        LOGGER.lock().push(Arc::new(logger));
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

    pub async fn log(&self, mut record: JsonLogRecord, msg: Option<Message>) -> anyhow::Result<()> {
        let _timer = self.submit_latency.start_timer();
        apply_classification(&mut record).await;
        match &self.implementation {
            LoggerImpl::Immediate(sig) => {
                match msg {
                    Some(msg) => {
                        // Need to put this future on the heap, otherwise we can consume
                        // too much stack
                        let future = Box::pin(DispHookParams::do_record(sig, msg, record));
                        future.await
                    }
                    None => Ok(()),
                }
            }
            LoggerImpl::Queue { sender, .. } => {
                match sender.try_send(LogCommand::Record(record, msg)) {
                    Ok(()) => Ok(()),
                    Err(TrySendError::Full(record)) => {
                        SUBMIT_FULL
                            .get_metric_with_label_values(&[&self.name])
                            .expect("get counter")
                            .inc();
                        sender.send_async(record).await?;
                        Ok(())
                    }
                    Err(TrySendError::Disconnected(_)) => anyhow::bail!("log channel was closed"),
                }
            }
        }
    }

    pub async fn signal_shutdown() {
        let loggers = Self::get_loggers();
        for logger in loggers.iter() {
            match &logger.implementation {
                LoggerImpl::Immediate(_) => {}
                LoggerImpl::Queue { sender, thread } => {
                    tracing::debug!("Terminating a logger");
                    sender.send_async(LogCommand::Terminate).await.ok();
                    tracing::debug!("Joining that logger");
                    let res = match thread.lock().await.take() {
                        Some(task) => Some(task.await),
                        None => None,
                    };
                    tracing::debug!("Joined -> {res:?}");
                }
            }
        }
    }

    pub fn extract_meta(&self, meta: &serde_json::Value) -> HashMap<String, Value> {
        let mut result = HashMap::new();

        for name in &self.meta {
            if let Some(prefix) = name.strip_suffix('*') {
                if let Some(obj) = meta.as_object() {
                    for (k, v) in obj {
                        if k.starts_with(prefix) {
                            result.insert(k.to_string(), v.clone());
                        }
                    }
                }
            } else if let Some(value) = meta.get(name) {
                if !value.is_null() {
                    result.insert(name.to_string(), value.clone());
                }
            }
        }

        result
    }

    pub async fn extract_fields(
        &self,
        msg: &Message,
    ) -> (HashMap<String, Value>, HashMap<String, Value>) {
        let mut headers = HashMap::new();

        if !self.headers.is_empty() {
            let mut all_headers: HashMap<String, (String, Vec<Value>)> = HashMap::new();
            for (name, value) in msg.get_all_headers().await.unwrap_or_else(|_| vec![]) {
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

        let meta = self.extract_meta(&msg.get_meta_obj().await.unwrap_or(serde_json::Value::Null));

        (headers, meta)
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "configure_bounce_classifier",
        lua.create_function(move |lua, params: LuaValue| {
            let params: ClassifierParams = from_lua_value(&lua, params)?;
            params.register().map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_local_logs",
        lua.create_async_function(|lua, params: LuaValue| async move {
            let params: LogFileParams = from_lua_value(&lua, params)?;
            Logger::init(params).await.map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_log_hook",
        lua.create_async_function(|lua, params: LuaValue| async move {
            let params: LogHookParams = from_lua_value(&lua, params)?;
            Logger::init_hook(params).await.map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_log_disposition_hook",
        lua.create_async_function(|lua, params: LuaValue| async move {
            let params: DispHookParams = from_lua_value(&lua, params)?;
            Logger::init_disp_hook(params).await.map_err(any_err)
        })?,
    )?;

    Ok(())
}
