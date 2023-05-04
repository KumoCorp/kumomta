use crate::delivery_metrics::DeliveryMetrics;
use crate::egress_path::EgressPathConfig;
use crate::egress_source::EgressSource;
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::lifecycle::{Activity, ShutdownSubcription};
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::lua_deliver::LuaQueueDispatcher;
use crate::queue::{DeliveryProto, Queue, QueueConfig, QueueManager};
use crate::runtime::{rt_spawn, rt_spawn_non_blocking, spawn};
use crate::smtp_dispatcher::SmtpDispatcher;
use crate::spool::SpoolManager;
use anyhow::Context;
use async_trait::async_trait;
use config::load_config;
use dns_resolver::MailExchanger;
use message::message::QueueNameComponents;
use message::Message;
use rfc5321::{EnhancedStatusCode, Response};
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, MutexGuard, Notify};
use tokio::task::JoinHandle;
use tracing::instrument; // TODO move to here

lazy_static::lazy_static! {
    static ref MANAGER: Mutex<ReadyQueueManager> = Mutex::new(ReadyQueueManager::new());
}

#[derive(Default)]
pub struct ReadyQueueManager {
    queues: HashMap<String, ReadyQueueHandle>,
}

impl ReadyQueueManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn number_of_queues(&self) -> usize {
        self.queues.len()
    }

    pub async fn get() -> MutexGuard<'static, Self> {
        MANAGER.lock().await
    }

    pub async fn get_opt(
        queue_name: &str,
        queue_config: &QueueConfig,
        egress_source: &str,
    ) -> Option<ReadyQueueHandle> {
        let components = QueueNameComponents::parse(queue_name);

        let needs_mx = matches!(&queue_config.protocol, DeliveryProto::Smtp);

        let mx = if needs_mx {
            Some(MailExchanger::resolve(components.domain).await.ok()?)
        } else {
            None
        };

        let site_name = mx
            .as_ref()
            .map(|mx| mx.site_name.to_string())
            .unwrap_or_else(|| components.domain.to_string());
        let name = format!("{egress_source}->{site_name}");

        let manager = Self::get().await;
        manager.queues.get(&name).cloned()
    }

    pub async fn resolve_by_queue_name(
        queue_name: &str,
        queue_config: &QueueConfig,
        egress_source: &str,
        egress_pool: &str,
    ) -> anyhow::Result<ReadyQueueHandle> {
        let components = QueueNameComponents::parse(queue_name);

        let needs_mx = matches!(&queue_config.protocol, DeliveryProto::Smtp);

        let mx = if needs_mx {
            Some(MailExchanger::resolve(components.domain).await?)
        } else {
            None
        };

        let site_name = mx
            .as_ref()
            .map(|mx| mx.site_name.to_string())
            .unwrap_or_else(|| components.domain.to_string());
        let name = format!("{egress_source}->{site_name}");

        let egress_source = EgressSource::resolve(egress_source)?;

        let mut config = load_config().await?;

        let path_config: EgressPathConfig = config
            .async_call_callback(
                "get_egress_path_config",
                (components.domain, egress_source.name.to_string(), site_name),
            )
            .await?;

        let mut manager = Self::get().await;
        let activity = Activity::get()?;

        let handle = manager.queues.entry(name.clone()).or_insert_with(|| {
            rt_spawn_non_blocking(format!("maintain {name}"), {
                let name = name.clone();
                move || Ok(async move { Self::maintainer_task(name).await })
            })
            .expect("failed to spawn maintainer");
            let service = format!("smtp_client:{name}");
            let metrics = DeliveryMetrics::new(&service, "smtp_client");
            let ready = Arc::new(StdMutex::new(VecDeque::new()));
            let notify = Arc::new(Notify::new());
            ReadyQueueHandle(Arc::new(Mutex::new(ReadyQueue {
                name: name.clone(),
                queue_name: queue_name.to_string(),
                ready,
                mx,
                notify,
                connections: vec![],
                last_change: Instant::now(),
                path_config,
                queue_config: queue_config.clone(),
                egress_source,
                metrics,
                activity,
                consecutive_connection_failures: Arc::new(AtomicUsize::new(0)),
                egress_pool: egress_pool.to_string(),
            })))
        });
        Ok(handle.clone())
    }

    async fn maintainer_task(name: String) -> anyhow::Result<()> {
        let mut shutdown = ShutdownSubcription::get();
        let mut interval = Duration::from_secs(60);
        let mut memory = crate::memory::subscribe_to_memory_status_changes();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {},
                _ = shutdown.shutting_down() => {
                    interval = Duration::from_secs(1);
                },
                _ = memory.changed() => {},
            };
            let mut mgr = Self::get().await;
            let queue = { mgr.queues.get(&name).cloned() };
            match queue {
                None => break,
                Some(queue) => {
                    let mut queue = queue.lock().await;
                    if queue.reapable().await {
                        tracing::debug!("reaping site {name}");
                        mgr.queues.remove(&name);
                        crate::metrics_helper::remove_metrics_for_service(&format!(
                            "smtp_client:{name}"
                        ));
                        break;
                    } else if crate::memory::get_headroom() == 0 {
                        queue.shrink_ready_queue_due_to_low_mem().await;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct ReadyQueueHandle(Arc<Mutex<ReadyQueue>>);

impl ReadyQueueHandle {
    pub async fn lock(&self) -> MutexGuard<ReadyQueue> {
        self.0.lock().await
    }
}

pub struct ReadyQueue {
    name: String,
    queue_name: String,
    ready: Arc<StdMutex<VecDeque<Message>>>,
    mx: Option<Arc<MailExchanger>>,
    notify: Arc<Notify>,
    connections: Vec<JoinHandle<()>>,
    last_change: Instant,
    metrics: DeliveryMetrics,
    activity: Activity,
    consecutive_connection_failures: Arc<AtomicUsize>,
    path_config: EgressPathConfig,
    queue_config: QueueConfig,
    egress_pool: String,
    egress_source: EgressSource,
}

impl ReadyQueue {
    #[allow(unused)]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn bounce_all(&mut self, bounce: &AdminBounceEntry) {
        let msgs: Vec<Message> = self.ready.lock().unwrap().drain(..).collect();
        self.metrics.ready_count.set(0);
        for msg in msgs {
            let id = *msg.id();
            bounce.log(msg, None).await;
            SpoolManager::remove_from_spool(id).await.ok();
        }
    }

    pub async fn insert(&mut self, msg: Message) -> Result<(), Message> {
        if crate::memory::low_memory() {
            msg.shrink().ok();
        }
        self.ready.lock().unwrap().push_back(msg);
        self.metrics.ready_count.inc();
        self.notify.notify_waiters();
        self.maintain().await;
        self.last_change = Instant::now();

        Ok(())
    }

    pub fn ready_count(&self) -> usize {
        self.ready.lock().unwrap().len()
    }

    pub fn ideal_connection_count(&self) -> usize {
        if self.activity.is_shutting_down() {
            0
        } else {
            let n = ideal_connection_count(self.ready_count(), self.path_config.connection_limit);
            if n > 0 && crate::memory::get_headroom() == 0 {
                n.min(2)
            } else {
                n
            }
        }
    }

    #[instrument(skip(self))]
    async fn shrink_ready_queue_due_to_low_mem(&mut self) {
        let mut ready = self.ready.lock().unwrap();
        ready.shrink_to_fit();
        if ready.is_empty() {
            return;
        }

        let mut count = 0;

        for msg in ready.iter() {
            if let Ok(true) = msg.shrink() {
                count += 1;
            }
        }

        tracing::error!(
            "did shrink {} of out {} msgs in ready queue {} due to memory shortage",
            count,
            ready.len(),
            self.name
        );
    }

    #[async_recursion::async_recursion]
    pub async fn maintain(&mut self) {
        // Prune completed connection tasks
        self.connections.retain(|handle| !handle.is_finished());

        if self.activity.is_shutting_down() {
            // We are shutting down; we want all messages to get saved.
            let msgs: Vec<Message> = self.ready.lock().unwrap().drain(..).collect();
            self.metrics.ready_count.set(0);
            if !msgs.is_empty() {
                let activity = self.activity.clone();
                spawn(format!("saving messages for {}", self.name), async move {
                    for msg in msgs {
                        Queue::save_if_needed_and_log(&msg).await;
                        drop(msg);
                    }
                    drop(activity);
                })
                .expect("failed to spawn save_if_needed_and_log");
            }

            return;
        }

        // TODO: throttle rate at which connections are opened
        let ideal = self.ideal_connection_count();

        for _ in self.connections.len()..ideal {
            // Open a new connection
            let name = self.name.clone();
            let queue_name = self.queue_name.clone();
            let mx = self.mx.clone();
            let ready = Arc::clone(&self.ready);
            let notify = self.notify.clone();
            let path_config = self.path_config.clone();
            let queue_config = self.queue_config.clone();
            let metrics = self.metrics.clone();
            let egress_source = self.egress_source.clone();
            let egress_pool = self.egress_pool.clone();
            let consecutive_connection_failures = self.consecutive_connection_failures.clone();

            tracing::trace!("spawning client for {name}");
            if let Ok(handle) = rt_spawn(format!("smtp client {name}"), move || {
                Ok(async move {
                    if let Err(err) = Dispatcher::run(
                        &name,
                        queue_name,
                        mx,
                        ready,
                        notify,
                        queue_config,
                        path_config,
                        metrics,
                        consecutive_connection_failures.clone(),
                        egress_source,
                        egress_pool,
                    )
                    .await
                    {
                        tracing::debug!(
                            "Error in Dispatcher::run for {name}: {err:#} \
                         (consecutive_connection_failures={consecutive_connection_failures:?})"
                        );
                    }
                })
            })
            .await
            {
                self.connections.push(handle);
            }
        }
    }

    pub async fn reapable(&mut self) -> bool {
        self.maintain().await;
        let ideal = self.ideal_connection_count();
        ideal == 0
            && self.connections.is_empty()
            && (self.last_change.elapsed() > Duration::from_secs(10 * 60))
                | self.activity.is_shutting_down()
    }
}

#[async_trait(?Send)]
pub trait QueueDispatcher: Debug + Send {
    async fn deliver_message(
        &mut self,
        message: Message,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()>;

    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()>;
    async fn have_more_connection_candidates(&mut self, dispatcher: &mut Dispatcher) -> bool;

    async fn close_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<bool>;
}

pub struct Dispatcher {
    pub name: String,
    pub queue_name: String,
    pub ready: Arc<StdMutex<VecDeque<Message>>>,
    pub notify: Arc<Notify>,
    pub path_config: EgressPathConfig,
    pub mx: Option<Arc<MailExchanger>>,
    pub metrics: DeliveryMetrics,
    pub shutting_down: ShutdownSubcription,
    pub activity: Activity,
    pub egress_source: EgressSource,
    pub egress_pool: String,
    pub delivered_this_connection: usize,
    pub msg: Option<Message>,
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        // Ensure that we re-queue any message that we had popped
        if let Some(msg) = self.msg.take() {
            let activity = self.activity.clone();
            rt_spawn_non_blocking("Dispatcher::drop".to_string(), move || {
                Ok(async move {
                    if activity.is_shutting_down() {
                        Queue::save_if_needed_and_log(&msg).await;
                    } else if let Err(err) = Dispatcher::requeue_message(msg, false, None).await {
                        tracing::error!("error requeuing message: {err:#}");
                    }
                })
            })
            .ok();
        }
    }
}

impl Dispatcher {
    #[instrument(skip(ready, metrics, notify))]
    async fn run(
        name: &str,
        queue_name: String,
        mx: Option<Arc<MailExchanger>>,
        ready: Arc<StdMutex<VecDeque<Message>>>,
        notify: Arc<Notify>,
        queue_config: QueueConfig,
        path_config: EgressPathConfig,
        metrics: DeliveryMetrics,
        consecutive_connection_failures: Arc<AtomicUsize>,
        egress_source: EgressSource,
        egress_pool: String,
    ) -> anyhow::Result<()> {
        let activity = Activity::get()?;
        let mut dispatcher = Self {
            name: name.to_string(),
            queue_name,
            ready,
            notify,
            mx,
            msg: None,
            path_config,
            metrics,
            shutting_down: ShutdownSubcription::get(),
            activity,
            egress_source,
            egress_pool,
            delivered_this_connection: 0,
        };

        let mut queue_dispatcher: Box<dyn QueueDispatcher> = match &queue_config.protocol {
            DeliveryProto::Smtp => match SmtpDispatcher::init(&mut dispatcher).await? {
                Some(disp) => Box::new(disp),
                None => return Ok(()),
            },
            DeliveryProto::Lua {
                custom_lua: proto_config,
            } => {
                let lua_config = load_config().await?;
                Box::new(LuaQueueDispatcher::new(lua_config, proto_config.clone()))
            }
            DeliveryProto::Maildir { .. } => {
                anyhow::bail!("Should not reach Dispatcher::run with DeliveryProto::Maildir")
            }
        };

        dispatcher.obtain_message();
        if dispatcher.msg.is_none() {
            // We raced with another dispatcher and there is no
            // more work to be done; no need to open a new connection.
            return Ok(());
        }

        let mut connection_failures = vec![];

        loop {
            if !dispatcher.wait_for_message(&mut *queue_dispatcher).await? {
                // No more messages within our idle time; we can close
                // the connection
                tracing::debug!("{} Idling out connection", dispatcher.name);
                return Ok(());
            }
            if let Err(err) = queue_dispatcher.attempt_connection(&mut dispatcher).await {
                connection_failures.push(format!("{err:#}"));
                dispatcher.metrics.connection_gauge.dec();
                dispatcher.metrics.global_connection_gauge.dec();
                if !queue_dispatcher
                    .have_more_connection_candidates(&mut dispatcher)
                    .await
                {
                    if let Some(msg) = dispatcher.msg.take() {
                        log_disposition(LogDisposition {
                            kind: RecordType::TransientFailure,
                            msg: msg.clone(),
                            site: &dispatcher.name,
                            peer_address: None,
                            response: Response {
                                code: 400,
                                enhanced_code: None,
                                content: format!(
                                    "KumoMTA internal: \
                                     failed to connect to any candidate \
                                     hosts: {}",
                                    connection_failures.join(", ")
                                ),
                                command: None,
                            },
                            egress_pool: Some(&dispatcher.egress_pool),
                            egress_source: Some(&dispatcher.egress_source.name),
                            relay_disposition: None,
                        })
                        .await;
                    }

                    if consecutive_connection_failures.fetch_add(1, Ordering::SeqCst)
                        > dispatcher
                            .path_config
                            .consecutive_connection_failures_before_delay
                    {
                        dispatcher.delay_ready_queue().await;
                    }
                    return Err(err);
                }
                tracing::debug!("{err:#}");
                // Try the next candidate MX address
                continue;
            }

            connection_failures.clear();
            consecutive_connection_failures.store(0, Ordering::SeqCst);
            dispatcher
                .deliver_message(&mut *queue_dispatcher)
                .await
                .context("deliver_message")?;
        }
    }

    #[instrument(skip(self))]
    async fn deliver_message(
        &mut self,
        queue_dispatcher: &mut dyn QueueDispatcher,
    ) -> anyhow::Result<()> {
        let msg = self.msg.as_ref().unwrap();

        msg.load_meta_if_needed().await?;
        msg.load_data_if_needed().await?;

        let activity = match Activity::get_opt() {
            Some(a) => a,
            None => {
                return Ok(());
            }
        };

        if let Some(throttle) = &self.path_config.max_message_rate {
            loop {
                let result = throttle
                    .throttle(format!("{}-message-rate", self.name))
                    .await?;

                if let Some(delay) = result.retry_after {
                    if delay >= self.path_config.client_timeouts.idle_timeout {
                        self.throttle_ready_queue(delay).await;
                        return Ok(());
                    }
                    tracing::trace!("{} throttled message rate, sleep for {delay:?}", self.name);
                    let mut shutdown = ShutdownSubcription::get();
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = shutdown.shutting_down() => {
                            anyhow::bail!("shutting down");
                        }
                    };
                } else {
                    break;
                }
            }
        }

        self.delivered_this_connection += 1;

        if let Err(err) = queue_dispatcher
            .deliver_message(self.msg.as_ref().unwrap().clone(), self)
            .await
        {
            // Transient failure; continue with another host
            tracing::debug!("failed to send message to {}: {err:#}", self.name,);
            return Err(err.into());
        }

        drop(activity);

        Ok(())
    }

    #[instrument(skip(msg))]
    pub async fn requeue_message(
        msg: Message,
        increment_attempts: bool,
        delay: Option<chrono::Duration>,
    ) -> anyhow::Result<()> {
        if !msg.is_meta_loaded() {
            msg.load_meta().await?;
        }
        let queue_name = msg.get_queue_name()?;
        let queue = QueueManager::resolve(&queue_name).await?;
        let mut queue = queue.lock().await;
        queue.requeue_message(msg, increment_attempts, delay).await
    }

    pub async fn throttle_ready_queue(&mut self, delay: Duration) {
        let mut msgs: Vec<Message> = self.ready.lock().unwrap().drain(..).collect();
        self.metrics.ready_count.set(0);
        if let Some(msg) = self.msg.take() {
            msgs.push(msg);
        }
        if !msgs.is_empty() {
            tracing::debug!(
                "throttled: delaying ready queue {} - {} messages",
                self.name,
                msgs.len()
            );
            let activity = self.activity.clone();
            let delay = chrono::Duration::from_std(delay).unwrap_or_else(|err| {
                tracing::error!(
                    "error creating duration from {delay:?}: {err:#}. Using 1 minute instead"
                );
                chrono::Duration::seconds(60)
            });
            rt_spawn("requeue for throttle".to_string(), move || {
                Ok(async move {
                    for msg in msgs {
                        if let Err(err) = Self::requeue_message(msg, false, Some(delay)).await {
                            tracing::error!("error requeuing message: {err:#}");
                        }
                    }
                    drop(activity);
                })
            })
            .await
            .expect("failed to spawn requeue");
        }
    }

    #[instrument(skip(self))]
    pub async fn bulk_ready_queue_operation(&mut self, response: Response) {
        let mut msgs: Vec<Message> = self.ready.lock().unwrap().drain(..).collect();
        self.metrics.ready_count.set(0);
        if let Some(msg) = self.msg.take() {
            msgs.push(msg);
        }
        if !msgs.is_empty() {
            let activity = self.activity.clone();
            let name = self.name.clone();
            let egress_pool = self.egress_pool.clone();
            let egress_source = self.egress_source.name.clone();
            rt_spawn(
                format!("bulk queue op for {} msgs {name} {response:?}", msgs.len()),
                move || {
                    Ok(async move {
                        let increment_attempts = true;
                        for msg in msgs {
                            log_disposition(LogDisposition {
                                kind: if response.is_transient() {
                                    RecordType::TransientFailure
                                } else {
                                    RecordType::Bounce
                                },
                                msg: msg.clone(),
                                site: &name,
                                peer_address: None,
                                response: response.clone(),
                                egress_pool: Some(&egress_pool),
                                egress_source: Some(&egress_source),
                                relay_disposition: None,
                            })
                            .await;

                            if response.is_transient() {
                                if let Err(err) =
                                    Self::requeue_message(msg, increment_attempts, None).await
                                {
                                    tracing::error!("error requeuing message: {err:#}");
                                }
                            } else if response.is_permanent() {
                                spawn("remove msg from spool", async move {
                                    SpoolManager::remove_from_spool(*msg.id()).await
                                })
                                .ok();
                            }
                        }
                        drop(activity);
                    })
                },
            )
            .await
            .expect("bulk queue spawned");
        }
    }

    #[instrument(skip(self))]
    async fn delay_ready_queue(&mut self) {
        tracing::debug!(
            "too many connection failures, delaying ready queue {}",
            self.name,
        );
        self.bulk_ready_queue_operation(Response {
            code: 451,
            enhanced_code: Some(EnhancedStatusCode {
                class: 4,
                subject: 4,
                detail: 1,
            }),
            content: "No answer from any hosts listed in MX".to_string(),
            command: None,
        })
        .await;
    }

    #[instrument(skip(self))]
    fn obtain_message(&mut self) -> bool {
        if self.msg.is_some() {
            return true;
        }
        self.msg = self.ready.lock().unwrap().pop_front();
        if self.msg.is_some() {
            self.metrics.ready_count.dec();
            true
        } else {
            false
        }
    }

    #[instrument(skip(self))]
    async fn wait_for_message(
        &mut self,
        queue_dispatcher: &mut dyn QueueDispatcher,
    ) -> anyhow::Result<bool> {
        if self.activity.is_shutting_down() {
            if let Some(msg) = self.msg.take() {
                Queue::save_if_needed_and_log(&msg).await;
            }
            return Ok(false);
        }

        if let Some(limit) = self.path_config.max_deliveries_per_connection {
            if self.delivered_this_connection >= limit {
                tracing::trace!(
                    "Sent {} and limit is {limit}, close and make a new connection",
                    self.delivered_this_connection
                );
                let closed = queue_dispatcher.close_connection(self).await?;
                if closed {
                    // Close out this dispatcher and let the maintainer spawn
                    // a new connection
                    return Ok(false);
                }
            }
        }

        if self.obtain_message() {
            return Ok(true);
        }

        let idle_timeout = self.path_config.client_timeouts.idle_timeout;
        tokio::select! {
            _ = tokio::time::sleep(idle_timeout) => {},
            _ = self.notify.notified() => {}
            _ = self.shutting_down.shutting_down() => {
                return Ok(false);
            }
        };
        Ok(self.obtain_message())
    }
}

/// Use an exponential decay curve in the increasing form, asymptotic up to connection_limit,
/// passes through 0.0, increasing but bounded to connection_limit.
///
/// Visualize on wolframalpha: "plot 32 * (1-exp(-x * 0.023)), x from 0 to 100, y from 0 to 32"
pub fn ideal_connection_count(queue_size: usize, connection_limit: usize) -> usize {
    let factor = 0.023;
    let goal = (connection_limit as f32) * (1. - (-1.0 * queue_size as f32 * factor).exp());
    goal.ceil() as usize
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn connection_limit() {
        let sizes = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 32, 64, 128, 256, 400, 512, 1024,
        ];
        let max_connections = 32;
        let targets: Vec<(usize, usize)> = sizes
            .iter()
            .map(|&queue_size| {
                (
                    queue_size,
                    ideal_connection_count(queue_size, max_connections),
                )
            })
            .collect();
        assert_eq!(
            vec![
                (0, 0),
                (1, 1),
                (2, 2),
                (3, 3),
                (4, 3),
                (5, 4),
                (6, 5),
                (7, 5),
                (8, 6),
                (9, 6),
                (10, 7),
                (20, 12),
                (32, 17),
                (64, 25),
                (128, 31),
                (256, 32),
                (400, 32),
                (512, 32),
                (1024, 32)
            ],
            targets
        );
    }
}
