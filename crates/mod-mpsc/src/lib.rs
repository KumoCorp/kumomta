use config::{any_err, get_or_create_sub_module};
use dashmap::DashMap;
use mlua::prelude::*;
use mlua::{Lua, UserDataMethods};
use mod_memoize::CacheValue;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{
    Receiver, Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel,
};
use tokio::time::Duration;

enum SenderWrapper {
    Bounded(Sender<CacheValue>),
    Unbounded(UnboundedSender<CacheValue>),
}

impl SenderWrapper {
    pub async fn send(&self, value: CacheValue) -> anyhow::Result<()> {
        match self {
            Self::Bounded(s) => {
                s.send(value).await?;
            }
            Self::Unbounded(s) => {
                s.send(value)?;
            }
        };
        Ok(())
    }

    pub fn try_send(&self, value: CacheValue) -> anyhow::Result<()> {
        match self {
            Self::Bounded(s) => {
                s.try_send(value)?;
            }
            Self::Unbounded(s) => {
                s.send(value)?;
            }
        };
        Ok(())
    }

    pub async fn send_timeout(&self, value: CacheValue, duration: Duration) -> anyhow::Result<()> {
        match self {
            Self::Bounded(s) => {
                s.send_timeout(value, duration).await?;
            }
            Self::Unbounded(s) => {
                s.send(value)?;
            }
        };
        Ok(())
    }
    pub fn is_closed(&self) -> bool {
        match self {
            Self::Bounded(r) => r.is_closed(),
            Self::Unbounded(r) => r.is_closed(),
        }
    }
}

enum ReceiverWrapper {
    Bounded(Receiver<CacheValue>),
    Unbounded(UnboundedReceiver<CacheValue>),
}

impl ReceiverWrapper {
    pub async fn recv(&mut self) -> Option<CacheValue> {
        match self {
            Self::Bounded(r) => r.recv().await,
            Self::Unbounded(r) => r.recv().await,
        }
    }

    pub fn try_recv(&mut self) -> Option<CacheValue> {
        match self {
            Self::Bounded(r) => r.try_recv().ok(),
            Self::Unbounded(r) => r.try_recv().ok(),
        }
    }

    pub async fn recv_many(&mut self, limit: usize) -> Vec<CacheValue> {
        let mut buffer = vec![];
        match self {
            Self::Bounded(r) => r.recv_many(&mut buffer, limit).await,
            Self::Unbounded(r) => r.recv_many(&mut buffer, limit).await,
        };

        buffer
    }

    pub fn close(&mut self) {
        match self {
            Self::Bounded(r) => r.close(),
            Self::Unbounded(r) => r.close(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Bounded(r) => r.is_empty(),
            Self::Unbounded(r) => r.is_empty(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Bounded(r) => r.len(),
            Self::Unbounded(r) => r.len(),
        }
    }
}

struct Queue {
    sender: Arc<SenderWrapper>,
    receiver: Arc<Mutex<ReceiverWrapper>>,
}

struct QueueHandle(Arc<Queue>);

impl LuaUserData for QueueHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method(
            "send",
            move |_lua, this: LuaUserDataRef<QueueHandle>, value: CacheValue| async move {
                this.0.sender.send(value).await.map_err(any_err)?;
                Ok(())
            },
        );

        methods.add_async_method(
            "send_timeout",
            move |_lua,
                  this: LuaUserDataRef<QueueHandle>,
                  (value, timeout_seconds): (CacheValue, f32)| async move {
                this.0
                    .sender
                    .send_timeout(value, Duration::from_secs_f32(timeout_seconds))
                    .await
                    .map_err(any_err)?;
                Ok(())
            },
        );

        methods.add_method(
            "try_send",
            move |_lua, this: &QueueHandle, value: CacheValue| match this.0.sender.try_send(value) {
                Ok(()) => Ok(true),
                Err(_) => Ok(false),
            },
        );

        methods.add_async_method(
            "close",
            move |_lua, this: LuaUserDataRef<QueueHandle>, ()| async move {
                let mut rx = this.0.receiver.lock().await;
                Ok(rx.close())
            },
        );

        methods.add_method("is_closed", move |_lua, this: &QueueHandle, ()| {
            Ok(this.0.sender.is_closed())
        });

        methods.add_async_method(
            "is_empty",
            move |_lua, this: LuaUserDataRef<QueueHandle>, ()| async move {
                let rx = this.0.receiver.lock().await;
                Ok(rx.is_empty())
            },
        );

        methods.add_async_method(
            "len",
            move |_lua, this: LuaUserDataRef<QueueHandle>, ()| async move {
                let rx = this.0.receiver.lock().await;
                Ok(rx.len())
            },
        );

        methods.add_async_method(
            "recv",
            move |_lua, this: LuaUserDataRef<QueueHandle>, ()| async move {
                let mut rx = this.0.receiver.lock().await;
                Ok(rx.recv().await)
            },
        );

        methods.add_async_method(
            "try_recv",
            move |_lua, this: LuaUserDataRef<QueueHandle>, ()| async move {
                let mut rx = this.0.receiver.lock().await;
                Ok(rx.try_recv())
            },
        );

        methods.add_async_method(
            "recv_many",
            move |_lua, this: LuaUserDataRef<QueueHandle>, limit: usize| async move {
                let mut rx = this.0.receiver.lock().await;
                Ok(rx.recv_many(limit).await)
            },
        );
    }
}

static QUEUES: LazyLock<DashMap<String, Arc<Queue>>> = LazyLock::new(DashMap::new);

impl Queue {
    pub fn define_unbounded(name: &str) -> anyhow::Result<QueueHandle> {
        let queue = QUEUES.entry(name.to_string()).or_insert_with(|| {
            let (sender, receiver) = unbounded_channel();
            Arc::new(Queue {
                sender: Arc::new(SenderWrapper::Unbounded(sender)),
                receiver: Arc::new(Mutex::new(ReceiverWrapper::Unbounded(receiver))),
            })
        });

        Ok(QueueHandle(Arc::clone(queue.value())))
    }

    pub fn define_bounded(name: &str, buffer: usize) -> anyhow::Result<QueueHandle> {
        let queue = QUEUES.entry(name.to_string()).or_insert_with(|| {
            let (sender, receiver) = channel(buffer);
            Arc::new(Queue {
                sender: Arc::new(SenderWrapper::Bounded(sender)),
                receiver: Arc::new(Mutex::new(ReceiverWrapper::Bounded(receiver))),
            })
        });

        Ok(QueueHandle(Arc::clone(queue.value())))
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mpsc = get_or_create_sub_module(lua, "mpsc")?;

    kumo_mpsc.set(
        "define",
        lua.create_function(|_lua, (name, buffer): (String, Option<usize>)| {
            match buffer {
                Some(buffer) => Queue::define_bounded(&name, buffer),
                None => Queue::define_unbounded(&name),
            }
            .map_err(any_err)
        })?,
    )?;

    Ok(())
}
