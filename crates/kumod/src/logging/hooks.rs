use crate::logging::files::LogFileParams;
use crate::logging::{LogCommand, LogRecordParams, LOGGING_RUNTIME};
use crate::queue::QueueManager;
use anyhow::Context;
use async_channel::Receiver;
use config::{load_config, CallbackSignature};
pub use kumo_log_types::*;
use message::{EnvelopeAddress, Message};
use minijinja::{Environment, Template};
use prometheus::CounterVec;
use serde::Deserialize;
use spool::SpoolId;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use tokio::sync::{Semaphore, TryAcquireError};

pub static SHOULD_ENQ_LOG_RECORD_SIG: LazyLock<CallbackSignature<(Message, String), bool>> =
    LazyLock::new(|| CallbackSignature::new_with_multiple("should_enqueue_log_record"));

static HOOK_BACKLOG_COUNT: LazyLock<CounterVec> = LazyLock::new(|| {
    prometheus::register_counter_vec!(
        "log_hook_backlog_count",
        "how many times processing of a log event hit the back_pressure in a hook",
        &["logger"]
    )
    .unwrap()
});

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

pub struct LogHookState {
    params: LogHookParams,
    receiver: Receiver<LogCommand>,
    template_engine: Environment<'static>,
    sema: Arc<Semaphore>,
}

impl LogHookState {
    pub fn new(
        params: LogHookParams,
        receiver: Receiver<LogCommand>,
        template_engine: Environment<'static>,
    ) -> Self {
        let sema = Arc::new(Semaphore::new(params.back_pressure));

        Self {
            params,
            receiver,
            template_engine,
            sema,
        }
    }

    pub async fn logger_thread(&mut self) {
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
                    if let Err(err) = self.do_record(record).await {
                        tracing::error!("failed to log: {err:#}");
                    };
                }
            }
        }
    }

    async fn do_record(&mut self, record: JsonLogRecord) -> anyhow::Result<()> {
        tracing::trace!("do_record {record:?}");

        if record.reception_protocol.as_deref() == Some("LogRecord") {
            return Ok(());
        }

        // Limit the concurrency for log hook dispatches.
        // We start synchronously (wrt. to acquiring the records) here,
        // but in the tail end of do_record we spawn a task to perform
        // the hook with parallelism. We don't want the number of outstanding
        // hook tasks to grow too large because:
        // 1. It is a sign that the logging system cannot keep up with
        //    the throughput of the system.
        // 2. If the system were to go down with a large backlog of unlogged
        //    items, there is increased risk that we won't have a record of
        //    what happened to the messages we processed.
        // 3. Unbounded growth increases system pressures which increases
        //    the risk of something going wrong.
        let permit = match self.sema.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(TryAcquireError::NoPermits) => {
                HOOK_BACKLOG_COUNT
                    .get_metric_with_label_values(&[&self.params.name])?
                    .inc();
                self.sema.clone().acquire_owned().await?
            }
            Err(TryAcquireError::Closed) => {
                anyhow::bail!("back_pressure semaphore is closed!?");
            }
        };

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

        LOGGING_RUNTIME.spawn_non_blocking("log-hook".to_string(), move || {
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
                drop(permit);
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
