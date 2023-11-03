use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::queue::QueueManager;
use crate::rt_spawn;
use anyhow::Context;
use chrono::Utc;
use config::{any_err, from_lua_value, get_or_create_module, CallbackSignature};
use kumo_server_lifecycle::{Activity, LifeCycle, ShutdownSubcription};
use kumo_server_runtime::spawn;
use message::Message;
use mlua::{Lua, Value};
use once_cell::sync::Lazy;
use rfc5321::{EnhancedStatusCode, Response};
use serde::Deserialize;
use spool::local_disk::LocalDiskSpool;
use spool::rocks::{RocksSpool, RocksSpoolParams};
use spool::{Spool as SpoolTrait, SpoolEntry, SpoolId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

static MANAGER: Lazy<SpoolManager> = Lazy::new(|| SpoolManager::new());

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
}

async fn define_spool(params: DefineSpoolParams) -> anyhow::Result<()> {
    crate::spool::SpoolManager::get()
        .await
        .new_local_disk(params)
        .await
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;
    kumo_mod.set(
        "define_spool",
        lua.create_async_function(|lua, params: Value| async move {
            let params = from_lua_value(lua, params)?;
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
    spooled_in: AtomicBool,
}

impl SpoolManager {
    pub fn new() -> Self {
        Self {
            named: Mutex::new(HashMap::new()),
            spooled_in: AtomicBool::new(false),
        }
    }

    pub async fn get() -> &'static Self {
        &MANAGER
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
                        LocalDiskSpool::new(&params.path, params.flush)
                            .with_context(|| format!("Opening spool {}", params.name))?,
                    ),
                    SpoolKind::RocksDB => Arc::new(
                        RocksSpool::new(&params.path, params.flush, params.rocks_params)
                            .with_context(|| format!("Opening spool {}", params.name))?,
                    ),
                },
            })),
        );
        Ok(())
    }

    #[allow(unused)]
    pub async fn get_named(name: &str) -> anyhow::Result<SpoolHandle> {
        Self::get().await.get_named_impl(name).await
    }

    pub async fn get_named_impl(&self, name: &str) -> anyhow::Result<SpoolHandle> {
        self.named
            .lock()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no spool named '{name}' has been defined"))
    }

    pub fn spool_started(&self) -> bool {
        self.spooled_in.load(Ordering::SeqCst)
    }

    pub async fn remove_from_spool(id: SpoolId) -> anyhow::Result<()> {
        let (data_spool, meta_spool) = {
            let mgr = Self::get().await;
            (
                mgr.get_named_impl("data").await?,
                mgr.get_named_impl("meta").await?,
            )
        };
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
        let data_spool = self.get_named_impl("data").await?;
        let meta_spool = self.get_named_impl("meta").await?;
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

    async fn spool_in_thread(
        &self,
        rx: flume::Receiver<SpoolEntry>,
        spooled_in: Arc<AtomicUsize>,
    ) -> anyhow::Result<()> {
        let mut shutdown = ShutdownSubcription::get();
        let mut config = config::load_config().await?;
        let egress_pool = None;
        let egress_source = None;

        let spool_message_enumerated =
            CallbackSignature::<Message, ()>::new("spool_message_enumerated");

        loop {
            let entry = tokio::select! {
                _ = shutdown.shutting_down() => anyhow::bail!("shutting down"),
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
                                    tracing::error!(
                                        "failed to resolve queue {queue_name}: {err:#}"
                                    );
                                }
                                Ok(queue) => {
                                    let mut queue = queue.lock().await;

                                    let queue_config = queue.get_config();
                                    let max_age = queue_config.borrow().get_max_age();
                                    let age = msg.age(now);
                                    let num_attempts =
                                        queue_config.borrow().infer_num_attempts(age);
                                    msg.set_num_attempts(num_attempts);

                                    match queue_config
                                        .borrow()
                                        .compute_delay_based_on_age(num_attempts, age)
                                    {
                                        None => {
                                            tracing::debug!("expiring {id} {age} > {max_age}");
                                            log_disposition(LogDisposition {
                                                kind: RecordType::Expiration,
                                                msg,
                                                site: "localhost",
                                                peer_address: None,
                                                response: Response {
                                                    code: 551,
                                                    enhanced_code: Some(EnhancedStatusCode {
                                                        class: 5,
                                                        subject: 4,
                                                        detail: 7,
                                                    }),
                                                    content: format!(
                                                        "Delivery time {age} > {max_age}"
                                                    ),
                                                    command: None,
                                                },
                                                egress_pool,
                                                egress_source,
                                                relay_disposition: None,
                                                delivery_protocol: None,
                                                tls_info: None,
                                            })
                                            .await;
                                            self.remove_from_spool_impl(id).await?;
                                            continue;
                                        }
                                        Some(delay) => {
                                            msg.delay_by(delay).await?;
                                        }
                                    }

                                    if let Err(err) = queue.insert(msg).await {
                                        tracing::error!(
                                            "failed to insert Message {id} \
                                             to queue {queue_name}: {err:#}"
                                        );
                                        self.remove_from_spool_impl(id).await?;
                                    }
                                }
                            },
                            Err(err) => {
                                tracing::error!(
                                    "Message {id} failed to compute queue name!: {err:#}"
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
                    tracing::error!("Failed to load {id}: {error}");
                    // TODO: log this better
                    self.remove_from_spool_impl(id).await?;
                }
            }
        }
    }

    pub async fn start_spool(&self) -> anyhow::Result<()> {
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
                                if let Err(err) = spool.enumerate(tx) {
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
        }

        // Ensure that there are no more senders outstanding,
        // otherwise we'll deadlock ourselves in the loop below
        drop(tx);

        let activity = Activity::get("spool enumeration".to_string())?;
        let spooled_in = Arc::new(AtomicUsize::new(0));
        tracing::debug!("start_spool: waiting for enumeration");
        let start = Instant::now();
        let interval = std::time::Duration::from_secs(30);

        let (complete_tx, complete_rx) = flume::bounded(1);

        let n_threads = (std::thread::available_parallelism()?.get() / 2).max(1);
        tracing::debug!("Using concurrency {n_threads} for spooling in");

        let mut num_tasks = 0;
        for n in 0..n_threads {
            let spooled_in = Arc::clone(&spooled_in);
            let rx = rx.clone();
            let complete_tx = complete_tx.clone();
            rt_spawn(format!("spool_in-{n}"), move || {
                Ok(async move {
                    let mgr = Self::get().await;
                    let result = mgr.spool_in_thread(rx, spooled_in).await;
                    complete_tx.send_async(result).await
                })
            })
            .await?;
            num_tasks += 1;
        }

        drop(complete_tx);

        while num_tasks > 0 {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {},
                _ = complete_rx.recv_async() => {
                    num_tasks -= 1;
                    continue;
                }
            };

            tracing::info!(
                "start_spool: still enumerating. {} items in {:?}",
                spooled_in.load(Ordering::SeqCst),
                start.elapsed()
            );
        }

        self.spooled_in.store(true, Ordering::SeqCst);
        let label = if activity.is_shutting_down() {
            "aborted"
        } else {
            "done"
        };
        drop(activity);
        tracing::info!(
            "start_spool: enumeration {label}, spooled in {} msgs over {:?}",
            spooled_in.load(Ordering::SeqCst),
            start.elapsed()
        );
        Ok(())
    }
}
