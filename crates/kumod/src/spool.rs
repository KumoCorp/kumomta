use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::{InsertReason, Queue, QueueManager};
use crate::smtp_server::ShuttingDownError;
use anyhow::Context;
use chrono::{DateTime, Utc};
use config::{any_err, from_lua_value, get_or_create_module, CallbackSignature};
use humansize::{format_size, DECIMAL};
use humantime::format_duration;
use kumo_server_common::disk_space::{MinFree, MonitoredPath};
use kumo_server_lifecycle::{Activity, LifeCycle, ShutdownSubcription};
use kumo_server_memory::subscribe_to_memory_status_changes_async;
use kumo_server_runtime::spawn;
use message::Message;
use mlua::{Lua, Value};
use rfc5321::{EnhancedStatusCode, Response};
use serde::Deserialize;
use spool::local_disk::LocalDiskSpool;
use spool::rocks::{RocksSpool, RocksSpoolParams};
use spool::{get_data_spool, get_meta_spool, Spool as SpoolTrait, SpoolEntry, SpoolId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

static MANAGER: LazyLock<SpoolManager> = LazyLock::new(SpoolManager::new);
static SPOOLIN_THREADS: AtomicUsize = AtomicUsize::new(0);

pub fn set_spoolin_threads(n: usize) {
    SPOOLIN_THREADS.store(n, Ordering::SeqCst);
}

#[derive(Clone)]
pub struct SpoolHandle(Arc<Spool>);

impl std::ops::Deref for SpoolHandle {
    type Target = dyn SpoolTrait + Send + Sync;
    fn deref(&self) -> &Self::Target {
        &*self.0.spool
    }
}

pub struct Spool {
    maintainer: StdMutex<Option<JoinHandle<()>>>,
    spool: Arc<dyn SpoolTrait + Send + Sync>,
}

impl std::ops::Deref for Spool {
    type Target = dyn SpoolTrait + Send + Sync;
    fn deref(&self) -> &Self::Target {
        &*self.spool
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        if let Some(handle) = self.maintainer.lock().unwrap().take() {
            handle.abort();
        }
    }
}

impl Spool {}

#[derive(Deserialize)]
pub enum SpoolKind {
    LocalDisk,
    RocksDB,
}
impl Default for SpoolKind {
    fn default() -> Self {
        Self::LocalDisk
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefineSpoolParams {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub kind: SpoolKind,
    #[serde(default)]
    pub flush: bool,
    #[serde(default)]
    pub rocks_params: Option<RocksSpoolParams>,

    #[serde(default)]
    pub min_free_space: MinFree,
    #[serde(default)]
    pub min_free_inodes: MinFree,
}

async fn define_spool(params: DefineSpoolParams) -> anyhow::Result<()> {
    MonitoredPath {
        name: format!("{} spool", params.name),
        path: params.path.clone(),
        min_free_space: params.min_free_space,
        min_free_inodes: params.min_free_inodes,
    }
    .register();

    crate::spool::SpoolManager::get()
        .new_local_disk(params)
        .await
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;
    kumo_mod.set(
        "define_spool",
        lua.create_async_function(|lua, params: Value| async move {
            let params = from_lua_value(&lua, params)?;
            if config::is_validating() {
                return Ok(());
            }

            spawn("define_spool", async move {
                if let Err(err) = define_spool(params).await {
                    tracing::error!("Error in spool: {err:#}");
                    LifeCycle::request_shutdown().await;
                }
            })
            .map_err(any_err)?
            .await
            .map_err(any_err)
        })?,
    )?;
    Ok(())
}

pub struct SpoolManager {
    named: Mutex<HashMap<String, SpoolHandle>>,
    started: AtomicBool,
}

impl SpoolManager {
    pub fn new() -> Self {
        Self {
            named: Mutex::new(HashMap::new()),
            started: AtomicBool::new(false),
        }
    }

    pub fn get() -> &'static Self {
        &MANAGER
    }

    async fn take() -> HashMap<String, SpoolHandle> {
        Self::get().named.lock().await.drain().collect()
    }

    pub async fn shutdown() -> anyhow::Result<()> {
        let named = Self::take().await;
        use tokio::task::JoinSet;
        let mut set: JoinSet<anyhow::Result<(String, Duration)>> = JoinSet::new();

        tracing::info!("Shutting down spool");
        for (name, handle) in named {
            set.spawn(async move {
                let start = Instant::now();
                handle
                    .shutdown()
                    .await
                    .with_context(|| format!("{name}: shutdown failed"))?;
                tokio::task::spawn_blocking(|| drop(handle))
                    .await
                    .with_context(|| format!("{name}: spawning drop failed"))?;
                Ok((name, start.elapsed()))
            });
        }

        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok((name, elapsed))) => {
                    tracing::info!("{name} shutdown completed in {elapsed:?}");
                }
                Ok(Err(err)) => {
                    tracing::error!("{err:#}");
                }
                Err(err) => {
                    tracing::error!("{err:#}");
                }
            }
        }

        Ok(())
    }

    pub async fn new_local_disk(&self, params: DefineSpoolParams) -> anyhow::Result<()> {
        tracing::debug!(
            "Defining local disk spool '{}' on {}",
            params.name,
            params.path.display()
        );
        self.named.lock().await.insert(
            params.name.to_string(),
            SpoolHandle(Arc::new(Spool {
                maintainer: StdMutex::new(None),
                spool: match params.kind {
                    SpoolKind::LocalDisk => Arc::new(
                        LocalDiskSpool::new(
                            &params.path,
                            params.flush,
                            kumo_server_runtime::get_main_runtime(),
                        )
                        .with_context(|| format!("Opening spool {}", params.name))?,
                    ),
                    SpoolKind::RocksDB => Arc::new(
                        RocksSpool::new(
                            &params.path,
                            params.flush,
                            params.rocks_params,
                            kumo_server_runtime::get_main_runtime(),
                        )
                        .with_context(|| format!("Opening spool {}", params.name))?,
                    ),
                },
            })),
        );
        Ok(())
    }

    #[allow(unused)]
    pub async fn get_named(name: &str) -> anyhow::Result<SpoolHandle> {
        Self::get().get_named_impl(name).await
    }

    pub async fn get_named_impl(&self, name: &str) -> anyhow::Result<SpoolHandle> {
        self.named
            .lock()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no spool named '{name}' has been defined"))
    }

    pub fn get_data_meta() -> (
        &'static Arc<dyn spool::Spool + Send + Sync>,
        &'static Arc<dyn spool::Spool + Send + Sync>,
    ) {
        (get_meta_spool(), get_data_spool())
    }

    pub fn spool_started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }

    pub async fn remove_from_spool(id: SpoolId) -> anyhow::Result<()> {
        let (data_spool, meta_spool) = Self::get_data_meta();
        let res_data = data_spool.remove(id).await;
        let res_meta = meta_spool.remove(id).await;
        if let Err(err) = res_data {
            // We don't log at error level for these because that
            // is undesirable when deferred_spool is enabled.
            tracing::debug!("Error removing data for {id}: {err:#}");
        }
        if let Err(err) = res_meta {
            tracing::debug!("Error removing meta for {id}: {err:#}");
        }
        Ok(())
    }

    pub async fn remove_from_spool_impl(&self, id: SpoolId) -> anyhow::Result<()> {
        let (data_spool, meta_spool) = Self::get_data_meta();
        let res_data = data_spool.remove(id).await;
        let res_meta = meta_spool.remove(id).await;
        if let Err(err) = res_data {
            tracing::debug!("Error removing data for {id}: {err:#}");
        }
        if let Err(err) = res_meta {
            tracing::debug!("Error removing meta for {id}: {err:#}");
        }
        Ok(())
    }

    /// Updates the next due time on msg, performing expiry if that due
    /// time is outside of either the per-message expiration configured
    /// via set_scheduling, or the max_age configured on the queue.
    /// If per-message expiration is configured, max_age is ignored.
    ///
    /// Returns Some(msg) if the message should be inserted into
    /// the queues, or None if the message was expired.
    async fn update_next_due(
        &self,
        id: SpoolId,
        msg: Message,
        queue: &Arc<Queue>,
        now: chrono::DateTime<Utc>,
    ) -> anyhow::Result<Option<Message>> {
        let queue_config = queue.get_config();
        let max_age = queue_config.borrow().get_max_age();
        let age = msg.age(now);
        let num_attempts = queue_config.borrow().infer_num_attempts(age);
        msg.set_num_attempts(num_attempts);

        match msg.get_scheduling().and_then(|sched| sched.expires) {
            Some(expires) => {
                // Per-message expiry
                let delay = queue_config
                    .borrow()
                    .compute_delay_based_on_age_ignoring_max_age(num_attempts, age);
                if let Some(next_due) = msg.delay_by(delay).await? {
                    if next_due >= expires {
                        tracing::debug!("expiring {id} {next_due} > scheduled expiry {expires}");
                        log_disposition(LogDisposition {
                            kind: RecordType::Expiration,
                            msg,
                            site: "",
                            peer_address: None,
                            response: Response {
                                code: 551,
                                enhanced_code: Some(EnhancedStatusCode {
                                    class: 5,
                                    subject: 4,
                                    detail: 7,
                                }),
                                content: format!(
                                    "Next delivery time would be at {next_due} \
                                    which exceeds the expiry time {expires} \
                                    configured via set_scheduling"
                                ),
                                command: None,
                            },
                            egress_pool: None,
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: None,
                            tls_info: None,
                            source_address: None,
                            provider: None,
                            session_id: None,
                        })
                        .await;
                        self.remove_from_spool_impl(id).await?;
                        return Ok(None);
                    }
                }
            }
            None => {
                // Regular queue based expiry
                match queue_config
                    .borrow()
                    .compute_delay_based_on_age(num_attempts, age)
                {
                    None => {
                        let age = format_duration(age.to_std().unwrap_or(Duration::ZERO));
                        let max_age = format_duration(max_age.to_std().unwrap_or(Duration::ZERO));
                        tracing::debug!("expiring {id} {age} > {max_age}");
                        log_disposition(LogDisposition {
                            kind: RecordType::Expiration,
                            msg,
                            site: "",
                            peer_address: None,
                            response: Response {
                                code: 551,
                                enhanced_code: Some(EnhancedStatusCode {
                                    class: 5,
                                    subject: 4,
                                    detail: 7,
                                }),
                                content: format!(
                                    "Next delivery time would be {age} \
                                    after creation, which exceeds max_age={max_age}"
                                ),
                                command: None,
                            },
                            egress_pool: None,
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: None,
                            tls_info: None,
                            source_address: None,
                            provider: None,
                            session_id: None,
                        })
                        .await;
                        self.remove_from_spool_impl(id).await?;
                        return Ok(None);
                    }
                    Some(delay) => {
                        msg.delay_by(delay).await?;
                    }
                }
            }
        }

        Ok(Some(msg))
    }

    async fn spool_in_thread(
        &self,
        rx: flume::Receiver<SpoolEntry>,
        spooled_in: Arc<AtomicUsize>,
        failed_spool_in: Arc<AtomicUsize>,
    ) -> anyhow::Result<()> {
        let mut shutdown = ShutdownSubcription::get();
        let mut config = config::load_config().await?;
        let egress_pool = None;
        let egress_source = None;

        let spool_message_enumerated =
            CallbackSignature::<Message, ()>::new("spool_message_enumerated");

        loop {
            let entry = tokio::select! {
                _ = shutdown.shutting_down() => return Err(ShuttingDownError.into()),
                entry = rx.recv_async() => { entry },
            }?;

            let now = Utc::now();
            match entry {
                SpoolEntry::Item { id, data } => match Message::new_from_spool(id, data) {
                    Ok(msg) => {
                        spooled_in.fetch_add(1, Ordering::SeqCst);

                        config
                            .async_call_callback(&spool_message_enumerated, msg.clone())
                            .await?;

                        match msg.get_queue_name() {
                            Ok(queue_name) => match QueueManager::resolve(&queue_name).await {
                                Err(err) => {
                                    // We don't remove from the spool in this case, because
                                    // it represents a general configuration error, not
                                    // a problem with this specific message
                                    tracing::error!(
                                        "failed to resolve queue {queue_name}: {err:#}. \
                                        Ignoring message until kumod is restarted."
                                    );
                                    failed_spool_in.fetch_add(1, Ordering::SeqCst);
                                }
                                Ok(queue) => {
                                    let Some(msg) =
                                        self.update_next_due(id, msg, &queue, now).await?
                                    else {
                                        // Expired
                                        continue;
                                    };

                                    if let Err(err) = queue
                                        .insert(msg.clone(), InsertReason::Enumerated.into(), None)
                                        .await
                                    {
                                        tracing::error!(
                                            "failed to insert Message {id} \
                                             to queue {queue_name}: {err:#}. \
                                             Ignoring message until kumod is restarted"
                                        );
                                        failed_spool_in.fetch_add(1, Ordering::SeqCst);
                                    }
                                }
                            },
                            Err(err) => {
                                // We delete the message in this case because a failure
                                // to create the queue name implies that the metadata
                                // for the message is somehow totally fubar: most likely
                                // due to some kind of corruption.
                                tracing::error!(
                                    "Message {id} failed to compute queue name!: {err:#}. \
                                    Removing message from the spool."
                                );
                                log_disposition(LogDisposition {
                                    kind: RecordType::Expiration,
                                    msg,
                                    site: "localhost",
                                    peer_address: None,
                                    response: Response {
                                        code: 551,
                                        enhanced_code: Some(EnhancedStatusCode {
                                            class: 5,
                                            subject: 1,
                                            detail: 3,
                                        }),
                                        content: format!("Failed to compute queue name: {err:#}"),
                                        command: None,
                                    },
                                    egress_pool,
                                    egress_source,
                                    relay_disposition: None,
                                    delivery_protocol: None,
                                    tls_info: None,
                                    source_address: None,
                                    provider: None,
                                    session_id: None,
                                })
                                .await;
                                self.remove_from_spool_impl(id).await?;
                            }
                        }
                    }
                    Err(err) => {
                        tracing::error!("Failed to parse metadata for {id}: {err:#}");
                        self.remove_from_spool_impl(id).await?;
                    }
                },
                SpoolEntry::Corrupt { id, error } => {
                    tracing::error!(
                        "Failed to load {id}: {error}. \
                        Removing message from the spool."
                    );
                    // TODO: log this better
                    self.remove_from_spool_impl(id).await?;
                }
            }
        }
    }

    pub async fn start_spool(&self, start_time: DateTime<Utc>) -> anyhow::Result<()> {
        self.started.store(true, Ordering::SeqCst);

        let (tx, rx) = flume::bounded(1024);
        {
            let mut named = self.named.lock().await;

            anyhow::ensure!(!named.is_empty(), "No spools have been defined");

            for (name, spool) in named.iter_mut() {
                let is_meta = name == "meta";

                match name.as_str() {
                    "meta" => spool::set_meta_spool(spool.0.spool.clone()),
                    "data" => spool::set_data_spool(spool.0.spool.clone()),
                    _ => {}
                }

                tracing::debug!("starting maintainer for spool {name} is_meta={is_meta}");

                let maintainer = kumo_server_runtime::spawn(format!("maintain spool {name}"), {
                    let name = name.clone();
                    let spool = spool.clone();
                    let tx = if is_meta { Some(tx.clone()) } else { None };
                    {
                        async move {
                            // start enumeration
                            if let Some(tx) = tx {
                                if let Err(err) = spool.enumerate(tx, start_time) {
                                    tracing::error!(
                                        "error during spool enumeration for {name}: {err:#}"
                                    );
                                }
                            }

                            // And maintain it every 10 minutes
                            loop {
                                tokio::time::sleep(Duration::from_secs(10 * 60)).await;
                                if let Err(err) = spool.cleanup().await {
                                    tracing::error!(
                                        "error doing spool cleanup for {name}: {err:#}"
                                    );
                                }
                            }
                        }
                    }
                })?;
                spool.0.maintainer.lock().unwrap().replace(maintainer);
            }

            Self::spawn_memory_monitor();
        }

        // Ensure that there are no more senders outstanding,
        // otherwise we'll deadlock ourselves in the loop below
        drop(tx);

        let activity = Activity::get("spool enumeration".to_string())?;
        let spooled_in = Arc::new(AtomicUsize::new(0));
        let failed_spool_in = Arc::new(AtomicUsize::new(0));
        tracing::debug!("start_spool: waiting for enumeration");
        let start = Instant::now();
        let interval = std::time::Duration::from_secs(30);

        let (complete_tx, complete_rx) = flume::bounded(1);

        let spooled_in_clone = Arc::clone(&spooled_in);
        let failed_spool_in_clone = Arc::clone(&failed_spool_in);

        let spool_in =
            kumo_server_runtime::Runtime::new("spoolin", |cpus| cpus / 2, &SPOOLIN_THREADS)?;

        for idx in 0..spool_in.get_num_threads() {
            spool_in.spawn(format!("spoolin-{idx}"), {
                let spooled_in = Arc::clone(&spooled_in_clone);
                let failed_spool_in = Arc::clone(&failed_spool_in_clone);
                let rx = rx.clone();
                let complete_tx = complete_tx.clone();
                async move {
                    let mgr = Self::get();
                    let result = mgr.spool_in_thread(rx, spooled_in, failed_spool_in).await;
                    complete_tx.send_async(result).await
                }
            })?;
        }
        let mut num_tasks = spool_in.get_num_threads();
        tracing::info!("Using concurrency {num_tasks} for spooling in");

        while num_tasks > 0 {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    let elapsed = start.elapsed();
                    let total =
                        spooled_in.load(Ordering::SeqCst);
                    let rate = (total as f64 / elapsed.as_secs_f64()).ceil() as u64;
                    tracing::info!(
                        "start_spool: still enumerating. {total} items in {elapsed:?} {rate}/s"
                    );
                }
                _ = complete_rx.recv_async() => {
                    num_tasks -= 1;
                }
            };
        }

        let label = if activity.is_shutting_down() {
            "aborted"
        } else {
            "done"
        };
        drop(activity);

        let elapsed = start.elapsed();
        let total = spooled_in.load(Ordering::SeqCst);
        let failed = failed_spool_in.load(Ordering::SeqCst);
        let rate = (total as f64 / elapsed.as_secs_f64()).ceil() as u64;
        tracing::info!(
            "start_spool: enumeration {label}, spooled in {total} msgs over {elapsed:?} {rate}/s"
        );
        if failed > 0 {
            tracing::error!(
                "start_spool: {failed}/{total} messages failed to spool in during enumeration. \
                These messages are NOT being processed and will remain in the spool until the \
                cause of the failure is addressed and kumod is restarted."
            );
        }

        // Move the Runtime to a non-async context so that we can drop it.
        // tokio's Runtime will panic if we don't do this.
        tokio::task::spawn_blocking(move || drop(spool_in)).await?;

        Ok(())
    }

    fn spawn_memory_monitor() {
        // Manage and trim memory usage
        tokio::spawn(async move {
            tracing::debug!("starting spool memory monitor");
            let mut memory_status = subscribe_to_memory_status_changes_async().await;
            while let Ok(()) = memory_status.changed().await {
                if kumo_server_memory::get_headroom() == 0 {
                    let mut spools = vec![];

                    {
                        let named = Self::get().named.lock().await;
                        for (name, spool) in named.iter() {
                            spools.push((name.clone(), spool.clone()));
                        }
                    }

                    for (name, spool) in spools {
                        match spool.advise_low_memory().await {
                            Ok(amount) if amount == 0 => {
                                tracing::error!("purge cache of {name}: no memory was reclaimed");
                            }
                            Ok(amount) if amount < 0 => {
                                tracing::error!(
                                    "purge cache of {name}: used additional {}",
                                    format_size((-amount) as usize, DECIMAL)
                                );
                            }
                            Ok(amount) => {
                                tracing::error!(
                                    "purge cache of {name}: saved {}",
                                    format_size(amount as usize, DECIMAL)
                                );
                            }
                            Err(err) => {
                                tracing::error!("purge cache of {name}: {err:#}");
                            }
                        }
                    }

                    // Wait a little bit so that we can debounce
                    // in the case where we're riding the cusp of
                    // the limit and would thrash the caches
                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                }
            }
        });
    }
}
