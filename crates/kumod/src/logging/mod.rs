use crate::logging::classify::{apply_classification, ClassifierParams};
use crate::logging::files::{LogFileParams, LogThreadState};
use crate::logging::hooks::{LogHookParams, LogHookState};
use anyhow::Context;
use async_channel::{Sender, TrySendError};
use config::{any_err, from_lua_value, get_or_create_module};
pub use kumo_log_types::*;
use kumo_server_common::disk_space::MonitoredPath;
use kumo_server_runtime::Runtime;
use message::Message;
use minijinja::Environment;
use minijinja_contrib::add_to_environment;
use mlua::{Lua, Value as LuaValue};
use parking_lot::FairMutex as Mutex;
use prometheus::{CounterVec, Histogram, HistogramVec};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinHandle;

pub(crate) mod classify;
pub(crate) mod disposition;
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
pub static LOGGING_RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("logging", |cpus| cpus / 4, &LOGGING_THREADS).unwrap());

pub fn set_logging_threads(n: usize) {
    LOGGING_THREADS.store(n, Ordering::SeqCst);
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

#[derive(Debug)]
pub(crate) enum LogCommand {
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
    hook_name: Option<String>,
    #[allow(unused)]
    name: String,
    submit_latency: Histogram,
}

impl Logger {
    fn get_loggers() -> Vec<Arc<Logger>> {
        LOGGER.lock().iter().map(Arc::clone).collect()
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
        let hook_name = params.name.to_string();
        let name = format!("hook-{hook_name}");
        let (sender, receiver) = async_channel::bounded(params.back_pressure);

        let mut state = LogHookState::new(params, receiver, template_engine);

        let thread = LOGGING_RUNTIME
            .spawn("log hook".to_string(), move || {
                Ok(async move {
                    tracing::debug!("calling state.logger_thread()");
                    state.logger_thread().await
                })
            })
            .await?;

        let submit_latency = SUBMIT_LATENCY.get_metric_with_label_values(&[&name])?;

        let logger = Self {
            sender,
            thread: TokioMutex::new(Some(thread)),
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
        let name = format!("dir-{}", params.log_dir.display());

        MonitoredPath {
            name: format!("log dir {}", params.log_dir.display()),
            path: params.log_dir.clone(),
            min_free_space: params.min_free_space,
            min_free_inodes: params.min_free_inodes,
        }
        .register();

        let thread = LOGGING_RUNTIME
            .spawn("log file".to_string(), move || {
                Ok(async move {
                    tracing::debug!("calling state.logger_thread()");
                    let mut state = LogThreadState {
                        params,
                        receiver,
                        template_engine,
                        file_map: HashMap::new(),
                    };
                    state.logger_thread().await
                })
            })
            .await?;

        let submit_latency = SUBMIT_LATENCY.get_metric_with_label_values(&[&name])?;

        let logger = Self {
            sender,
            thread: TokioMutex::new(Some(thread)),
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

    pub async fn log(&self, mut record: JsonLogRecord) -> anyhow::Result<()> {
        let _timer = self.submit_latency.start_timer();
        apply_classification(&mut record).await;
        match self.sender.try_send(LogCommand::Record(record)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(record)) => {
                SUBMIT_FULL
                    .get_metric_with_label_values(&[&self.name])
                    .expect("get counter")
                    .inc();
                self.sender.send(record).await?;
                Ok(())
            }
            Err(TrySendError::Closed(_)) => anyhow::bail!("log channel was closed"),
        }
    }

    pub fn signal_shutdown() -> Pin<Box<dyn Future<Output = ()>>> {
        Box::pin(async move {
            let loggers = Self::get_loggers();
            for logger in loggers.iter() {
                tracing::debug!("Terminating a logger");
                logger.sender.send(LogCommand::Terminate).await.ok();
                tracing::debug!("Joining that logger");
                let res = match logger.thread.lock().await.take() {
                    Some(task) => Some(task.await),
                    None => None,
                };
                tracing::debug!("Joined -> {res:?}");
            }
        })
    }

    pub fn extract_meta(&self, meta: &serde_json::Value) -> HashMap<String, Value> {
        let mut result = HashMap::new();

        for name in &self.meta {
            if let Some(value) = meta.get(name) {
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

        let meta = self.extract_meta(&msg.get_meta_obj().unwrap_or(serde_json::Value::Null));

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

    Ok(())
}
