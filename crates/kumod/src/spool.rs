use crate::queue::QueueManager;
use message::Message;
use spool::local_disk::LocalDiskSpool;
use spool::{Spool as SpoolTrait, SpoolEntry};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;

lazy_static::lazy_static! {
    pub static ref MANAGER: Mutex<SpoolManager> = Mutex::new(SpoolManager::new());
}

#[derive(Clone)]
pub struct SpoolHandle(Arc<Mutex<Spool>>);

impl SpoolHandle {
    pub async fn lock(&self) -> MutexGuard<Spool> {
        self.0.lock().await
    }
}

pub struct Spool {
    maintainer: Option<JoinHandle<()>>,
    spool: Box<dyn SpoolTrait + Send + Sync>,
}

impl std::ops::Deref for Spool {
    type Target = dyn SpoolTrait + Send + Sync;
    fn deref(&self) -> &Self::Target {
        &*self.spool
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        if let Some(handle) = self.maintainer.take() {
            handle.abort();
        }
    }
}

impl Spool {}

pub struct SpoolManager {
    named: HashMap<String, SpoolHandle>,
    spooled_in: bool,
}

impl SpoolManager {
    pub fn new() -> Self {
        Self {
            named: HashMap::new(),
            spooled_in: false,
        }
    }

    pub async fn get() -> MutexGuard<'static, Self> {
        MANAGER.lock().await
    }

    pub fn new_local_disk(&mut self, name: &str, path: &Path) -> anyhow::Result<()> {
        tracing::debug!("Defining local disk spool {name}");
        self.named.insert(
            name.to_string(),
            SpoolHandle(Arc::new(Mutex::new(Spool {
                maintainer: None,
                spool: Box::new(LocalDiskSpool::new(path)?),
            }))),
        );
        Ok(())
    }

    pub async fn get_named(name: &str) -> anyhow::Result<SpoolHandle> {
        Self::get()
            .await
            .named
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no spool named '{name}' has been defined"))
    }

    pub fn spool_started(&self) -> bool {
        self.spooled_in
    }

    pub async fn start_spool(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.named.is_empty(), "No spools have been defined");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        for (name, spool) in self.named.iter_mut() {
            let is_meta = name == "meta";

            tracing::debug!("starting maintainer for spool {name} is_meta={is_meta}");

            let maintainer = tokio::spawn({
                let name = name.clone();
                let spool = spool.clone();
                let tx = if is_meta { Some(tx.clone()) } else { None };
                {
                    async move {
                        // start enumeration
                        if let Some(tx) = tx {
                            let spool = spool.lock().await;
                            if let Err(err) = spool.spool.enumerate(tx) {
                                tracing::error!(
                                    "error during spool enumeration for {name}: {err:#}"
                                );
                            }
                        }

                        // And maintain it every 10 minutes
                        loop {
                            tokio::time::sleep(Duration::from_secs(10 * 60)).await;
                            let spool = spool.lock().await;
                            if let Err(err) = spool.spool.cleanup().await {
                                tracing::error!("error doing spool cleanup for {name}: {err:#}");
                            }
                        }
                    }
                }
            });
            spool.lock().await.maintainer.replace(maintainer);
        }

        // Ensure that there are no more senders outstanding,
        // otherwise we'll deadlock ourselves in the loop below
        drop(tx);

        tracing::debug!("start_spool: waiting for enumeration");
        while let Some(entry) = rx.recv().await {
            match entry {
                SpoolEntry::Item { id, data } => {
                    match Message::new_from_spool(id, data) {
                        Ok(msg) => {
                            let mut queue_manager = QueueManager::get().await;
                            match msg.recipient() {
                                Ok(recip) => {
                                    let domain = recip.domain().to_string();
                                    if let Err(err) = queue_manager.insert(&domain, msg).await {
                                        tracing::error!(
                                            "failed to insert Message {id} to queue: {err:#}"
                                        );
                                        // TODO: remove it from the spool here?
                                    }
                                }
                                Err(err) => {
                                    tracing::error!(
                                        "Message {id} is missing a recipient!: {err:#}"
                                    );
                                    // TODO: remove it from the spool here?
                                }
                            }
                        }
                        Err(err) => {
                            tracing::error!("Failed to parse metadata for {id}: {err:#}");
                            // TODO: remove it from the spool here?
                        }
                    }
                }
                SpoolEntry::Corrupt { id, error } => {
                    tracing::error!("Failed to load {id}: {error}");
                    // TODO: log this better
                    // TODO: remove it from the spool here?
                }
            }
        }
        self.spooled_in = true;
        tracing::debug!("start_spool: enumeration done");
        Ok(())
    }
}
