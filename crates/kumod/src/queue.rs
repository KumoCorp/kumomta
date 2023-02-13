use message::Message;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use timeq::{TimeQ, TimerError};
use tokio::task::JoinHandle;

lazy_static::lazy_static! {
    pub static ref MANAGER: Mutex<QueueManager> = Mutex::new(QueueManager::new());
}

#[derive(Clone)]
pub struct QueueHandle(Arc<Mutex<Queue>>);

impl QueueHandle {
    pub fn lock(&self) -> MutexGuard<Queue> {
        self.0.lock().unwrap()
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
    pub fn new(name: String) -> QueueHandle {
        let handle = QueueHandle(Arc::new(Mutex::new(Queue {
            name: name.clone(),
            queue: TimeQ::new(),
            maintainer: None,
        })));

        let queue_clone = handle.clone();
        let maintainer = tokio::spawn(async move {
            if let Err(err) = maintain_named_queue(&queue_clone).await {
                tracing::error!("maintain_named_queue {}: {err:#}", queue_clone.lock().name);
            }
        });
        handle.lock().maintainer.replace(maintainer);
        handle
    }

    pub fn insert(&mut self, msg: Message) {
        match self.queue.insert(Arc::new(msg)) {
            Ok(_) => {}
            Err(TimerError::Expired(msg)) => {
                // TODO: for immediately ready messages,
                // immediately instantiate the destination site
                // and add to its queue
                tracing::error!("queue to destination site");
            }
            Err(TimerError::NotFound) => unreachable!(),
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
    pub fn insert(&mut self, name: &str, msg: Message) {
        let name = name.to_lowercase();
        let queue = self
            .named
            .entry(name.clone())
            .or_insert_with(|| Queue::new(name));
        queue.lock().insert(msg);
    }

    pub fn get() -> MutexGuard<'static, Self> {
        MANAGER.lock().unwrap()
    }
}

async fn maintain_named_queue(queue: &QueueHandle) -> anyhow::Result<()> {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        {
            let q = queue.lock();
            println!(
                "maintaining queue {} which has {} entries",
                q.name,
                q.queue.len()
            );
        }
    }
}
