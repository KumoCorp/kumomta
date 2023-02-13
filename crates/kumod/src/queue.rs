use crate::dest_site::SiteManager;
use message::Message;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use timeq::{TimeQ, TimerError};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;

lazy_static::lazy_static! {
    pub static ref MANAGER: Mutex<QueueManager> = Mutex::new(QueueManager::new());
}

#[derive(Clone)]
pub struct QueueHandle(Arc<Mutex<Queue>>);

impl QueueHandle {
    pub async fn lock(&self) -> MutexGuard<Queue> {
        self.0.lock().await
    }
}

pub struct Queue {
    name: String,
    queue: TimeQ<Message>,
    maintainer: Option<JoinHandle<()>>,
}

impl Drop for Queue {
    fn drop(&mut self) {
        if let Some(handle) = self.maintainer.take() {
            handle.abort();
        }
    }
}

impl Queue {
    pub async fn new(name: String) -> QueueHandle {
        let handle = QueueHandle(Arc::new(Mutex::new(Queue {
            name: name.clone(),
            queue: TimeQ::new(),
            maintainer: None,
        })));

        let queue_clone = handle.clone();
        let maintainer = tokio::spawn(async move {
            if let Err(err) = maintain_named_queue(&queue_clone).await {
                tracing::error!(
                    "maintain_named_queue {}: {err:#}",
                    queue_clone.lock().await.name
                );
            }
        });
        handle.lock().await.maintainer.replace(maintainer);
        handle
    }

    pub async fn insert(&mut self, msg: Message) -> anyhow::Result<()> {
        match self.queue.insert(Arc::new(msg)) {
            Ok(_) => Ok(()),
            Err(TimerError::Expired(msg)) => {
                let msg = (*msg).clone();
                match SiteManager::resolve_domain(&self.name).await {
                    Ok(site) => {
                        let site = site.lock().await;
                        println!("site is {}", site.name());
                        match site.insert(msg) {
                            Ok(_) => {}
                            Err(TrySendError::Closed(msg)) | Err(TrySendError::Full(msg)) => {
                                msg.delay_by(Duration::from_secs(60));
                                self.queue
                                    .insert(Arc::new(msg))
                                    .map_err(|_err| anyhow::anyhow!("failed to insert"))?;
                            }
                        }
                    }
                    Err(err) => {
                        tracing::error!("Failed to resolve {}: {err:#}", self.name);
                        msg.delay_by(Duration::from_secs(60));
                        self.queue
                            .insert(Arc::new(msg))
                            .map_err(|_err| anyhow::anyhow!("failed to insert"))?;
                    }
                }

                Ok(())
            }
            Err(TimerError::NotFound) => {
                anyhow::bail!("queue.insert returned impossible NotFound error")
            }
        }
    }
}

pub struct QueueManager {
    named: HashMap<String, QueueHandle>,
}

impl QueueManager {
    pub fn new() -> Self {
        Self {
            named: HashMap::new(),
        }
    }

    /// Insert message into a queue named `name`.
    /// Note that the queue names are case-insensitive, and
    /// internally the lowercased version of `name` is used
    /// to track the queue.
    pub async fn insert(&mut self, name: &str, msg: Message) -> anyhow::Result<()> {
        let name = name.to_lowercase();
        let entry_keeper;
        let entry = match self.named.get(&name) {
            Some(e) => e,
            None => {
                entry_keeper = Queue::new(name.clone()).await;
                self.named.insert(name, entry_keeper.clone());
                &entry_keeper
            }
        };
        let mut entry = entry.lock().await;
        entry.insert(msg).await
    }

    pub async fn get() -> MutexGuard<'static, Self> {
        MANAGER.lock().await
    }
}

async fn maintain_named_queue(queue: &QueueHandle) -> anyhow::Result<()> {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        {
            let q = queue.lock().await;
            println!(
                "maintaining queue {} which has {} entries",
                q.name,
                q.queue.len()
            );
        }
    }
}
