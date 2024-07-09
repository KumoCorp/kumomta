use crate::delivery_metrics::DeliveryMetrics;
use crate::egress_source::EgressSource;
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::http_server::admin_suspend_ready_q_v1::{
    AdminSuspendReadyQEntry, AdminSuspendReadyQEntryRef,
};
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::lua_deliver::LuaQueueDispatcher;
use crate::queue::{DeliveryProto, Queue, QueueConfig, QueueManager, QMAINT_RUNTIME};
use crate::smtp_dispatcher::{MxListEntry, SmtpDispatcher};
use crate::spool::SpoolManager;
use anyhow::Context;
use async_trait::async_trait;
use config::{load_config, CallbackSignature};
use crossbeam_queue::ArrayQueue;
use dns_resolver::MailExchanger;
use kumo_api_types::egress_path::EgressPathConfig;
use kumo_server_common::config_handle::ConfigHandle;
use kumo_server_lifecycle::{Activity, ShutdownSubcription};
use kumo_server_memory::{get_headroom, low_memory, subscribe_to_memory_status_changes};
use kumo_server_runtime::{spawn, Runtime};
use message::message::QueueNameComponents;
use message::Message;
use parking_lot::FairMutex as StdMutex;
use prometheus::IntGauge;
use rfc5321::{EnhancedStatusCode, Response};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use throttle::limit::{LimitLease, LimitSpec};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::instrument; // TODO move to here

lazy_static::lazy_static! {
    static ref MANAGER: StdMutex<ReadyQueueManager> = StdMutex::new(ReadyQueueManager::new());
    pub static ref REQUEUE_MESSAGE_SIG: CallbackSignature::<'static,
        Message, ()> = CallbackSignature::new_with_multiple("message_requeued");
    pub static ref READYQ_RUNTIME: Runtime = Runtime::new(
        "readyq", |cpus| cpus / 2, &READYQ_THREADS).unwrap();
}

static READYQ_THREADS: AtomicUsize = AtomicUsize::new(0);

pub fn set_readyq_threads(n: usize) {
    READYQ_THREADS.store(n, Ordering::SeqCst);
}

pub struct Fifo {
    queue: ArrayQueue<Message>,
    count: IntGauge,
}

impl Fifo {
    pub fn new(capacity: usize, count: IntGauge) -> Self {
        Self {
            queue: ArrayQueue::new(capacity),
            count,
        }
    }

    pub fn push(&self, msg: Message) -> Result<(), Message> {
        self.queue.push(msg)?;
        self.count.inc();
        Ok(())
    }

    pub fn pop(&self) -> Option<Message> {
        let msg = self.queue.pop()?;
        self.count.dec();
        Some(msg)
    }

    pub fn drain(&self) -> Vec<Message> {
        let mut messages = Vec::with_capacity(self.queue.len());
        while let Some(msg) = self.queue.pop() {
            messages.push(msg);
        }
        self.count.sub(messages.len() as i64);
        messages
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

pub struct ReadyQueueName {
    pub name: String,
    pub site_name: String,
    pub mx: Option<Arc<MailExchanger>>,
}

impl ReadyQueueName {
    pub fn has_expired(&self) -> bool {
        match &self.mx {
            Some(mx) => mx.has_expired(),
            None => false,
        }
    }
}

pub struct ReadyQueueConfig {
    pub name: String,
    #[allow(unused)]
    pub site_name: String,
    pub path_config: EgressPathConfig,
    pub egress_source: EgressSource,
    pub mx: Option<Arc<MailExchanger>>,
}

#[derive(Default)]
pub struct ReadyQueueManager {
    queues: HashMap<String, ReadyQueueHandle>,
}

impl ReadyQueueManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn number_of_queues() -> usize {
        MANAGER.lock().queues.len()
    }

    pub async fn compute_queue_name(
        queue_name: &str,
        queue_config: &ConfigHandle<QueueConfig>,
        egress_source: &str,
    ) -> anyhow::Result<ReadyQueueName> {
        let components = QueueNameComponents::parse(queue_name);
        let mut mx = None;

        let routing_domain = components
            .routing_domain
            .as_deref()
            .unwrap_or(&components.domain);

        // Note well! The ready queue name is based on the perspective of the
        // receiver, combining our source (which they see) with the unique
        // name of the destination (which we compute from the MX site name).
        // For custom protocols where we don't have an MX record and thus no
        // site name, we simply use the domain; we do not include the campaign
        // or tenant because those have no bearing from the perspective of
        // the recipient.
        let site_name = match &queue_config.borrow().protocol {
            DeliveryProto::Smtp { smtp } => {
                if smtp.mx_list.is_empty() {
                    mx.replace(MailExchanger::resolve(routing_domain).await?);
                    mx.as_ref().unwrap().site_name.to_string()
                } else {
                    let mut mx_list = vec![];
                    for a in smtp.mx_list.iter() {
                        match a {
                            MxListEntry::Name(a) => {
                                mx_list.push(a.clone());
                            }
                            MxListEntry::Resolved(addr) => {
                                mx_list.push(addr.addr.to_string());
                            }
                        }
                    }
                    format!("mx_list:{}", mx_list.join(","))
                }
            }
            _ => routing_domain.to_string(),
        };

        // Factor in the delivery protocol so that we don't falsely share
        // different custom protocols when someone is using eg: just the
        // tenant or campaign to vary the protocol.
        // NOTE: this is coupled with the logic in
        // tsa-daemon/src/http_server that reverses/extracts the site_name
        // portion of this string.
        // If you change this then you must update that other logic accordingly.
        let name = format!(
            "{egress_source}->{site_name}@{}",
            queue_config.borrow().protocol.ready_queue_name()
        );

        Ok(ReadyQueueName {
            name,
            site_name,
            mx,
        })
    }

    async fn compute_config(
        queue_name: &str,
        queue_config: &ConfigHandle<QueueConfig>,
        egress_source: &str,
    ) -> anyhow::Result<ReadyQueueConfig> {
        let ReadyQueueName {
            name,
            site_name,
            mx,
        } = Self::compute_queue_name(queue_name, queue_config, egress_source).await?;

        let components = QueueNameComponents::parse(queue_name);
        let routing_domain = components
            .routing_domain
            .as_deref()
            .unwrap_or(&components.domain);

        let mut config = load_config().await?;

        let egress_source = EgressSource::resolve(egress_source, &mut config).await?;

        let sig = CallbackSignature::<(&str, String, String), EgressPathConfig>::new(
            "get_egress_path_config",
        );

        let path_config: EgressPathConfig = config
            .async_call_callback(
                &sig,
                (
                    routing_domain,
                    egress_source.name.to_string(),
                    site_name.clone(),
                ),
            )
            .await
            .map_err(|err| {
                tracing::error!("Error while calling get_egress_path_config: {err:#}");
                err
            })?;

        Ok(ReadyQueueConfig {
            name,
            site_name,
            path_config,
            egress_source,
            mx,
        })
    }

    pub fn get_by_name(name: &str) -> Option<ReadyQueueHandle> {
        let manager = MANAGER.lock();
        manager.queues.get(name).cloned()
    }

    pub fn get_by_ready_queue_name(name: &ReadyQueueName) -> Option<ReadyQueueHandle> {
        Self::get_by_name(&name.name)
    }

    pub async fn resolve_by_queue_name(
        queue_name: &str,
        queue_config: &ConfigHandle<QueueConfig>,
        egress_source: &str,
        egress_pool: &str,
    ) -> anyhow::Result<ReadyQueueHandle> {
        let ReadyQueueConfig {
            name,
            site_name: _,
            path_config,
            egress_source,
            mx,
        } = Self::compute_config(queue_name, queue_config, egress_source).await?;

        let mut manager = MANAGER.lock();
        let activity = Activity::get(format!("ReadyQueueHandle {name}"))?;

        let handle = manager.queues.entry(name.clone()).or_insert_with(|| {
            let notify_maintainer = Arc::new(Notify::new());
            QMAINT_RUNTIME
                .spawn_non_blocking(format!("maintain {name}"), {
                    let name = name.clone();
                    let notify_maintainer = notify_maintainer.clone();
                    move || Ok(async move { Self::maintainer_task(name, notify_maintainer).await })
                })
                .expect("failed to spawn maintainer");
            let proto = queue_config.borrow().protocol.metrics_protocol_name();
            let service = format!("{proto}:{name}");
            let metrics = DeliveryMetrics::new(&service, &proto);
            let ready = Arc::new(Fifo::new(
                path_config.max_ready,
                metrics.ready_count.clone(),
            ));
            let notify_dispatcher = Arc::new(Notify::new());
            Arc::new(ReadyQueue {
                name: name.clone(),
                queue_name_for_config_change_purposes_only: queue_name.to_string(),
                ready,
                mx,
                notify_dispatcher,
                notify_maintainer,
                connections: StdMutex::new(vec![]),
                path_config: ConfigHandle::new(path_config),
                queue_config: queue_config.clone(),
                egress_source,
                metrics,
                activity,
                consecutive_connection_failures: Arc::new(AtomicUsize::new(0)),
                egress_pool: egress_pool.to_string(),
            })
        });
        Ok(handle.clone())
    }

    async fn maintainer_task(name: String, notify_maintainer: Arc<Notify>) -> anyhow::Result<()> {
        const ONE_MINUTE: Duration = Duration::from_secs(60);
        let mut shutdown = ShutdownSubcription::get();
        let mut interval = ONE_MINUTE;
        let mut memory = subscribe_to_memory_status_changes();
        let mut last_change = Instant::now();
        let mut last_config_refresh = tokio::time::Instant::now();
        let mut reap_deadline = None;
        let mut done_abort = false;

        let queue = ReadyQueueManager::get_by_name(&name).ok_or_else(|| {
            anyhow::anyhow!("ready_queue {name} not found when starting up maintainer_task")
        })?;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(last_config_refresh + interval) => {
                },
                _ = shutdown.shutting_down() => {
                    interval = Duration::from_secs(1);
                    if reap_deadline.is_none() {
                        let duration = queue.path_config.borrow().client_timeouts.total_message_send_duration();
                        reap_deadline.replace(tokio::time::Instant::now() + duration);
                        tracing::debug!("{name}: reap deadline in {duration:?}");
                    }
                },
                _ = memory.changed() => {},
                _ = notify_maintainer.notified() => {
                    last_change = Instant::now();
                },
            };

            if last_config_refresh.elapsed() >= ONE_MINUTE && !queue.activity.is_shutting_down() {
                last_config_refresh = tokio::time::Instant::now();
                match Self::compute_config(
                    &queue.queue_name_for_config_change_purposes_only,
                    &queue.queue_config,
                    &queue.egress_source.name,
                )
                .await
                {
                    Ok(ReadyQueueConfig { path_config, .. }) => {
                        if path_config != **queue.path_config.borrow() {
                            let generation = queue.path_config.update(path_config);
                            // Note that the Fifo type doesn't allow for dynamically
                            // changing the capacity of the ready queue, so you will
                            // need to allow the ready queue to be reaped before that
                            // change takes effect
                            tracing::trace!("{name}: refreshed get_egress_path_config to generation {generation}");
                            queue.notify_dispatcher.notify_waiters();
                        }
                    }
                    Err(err) => {
                        tracing::error!("{name}: refreshing get_egress_path_config: {err:#}");
                    }
                }
            }

            let suspend = AdminSuspendReadyQEntry::get_for_queue_name(&name);
            queue.maintain(&suspend).await;

            if queue.reapable(&last_change, &suspend) {
                let mut mgr = MANAGER.lock();
                if queue.reapable(&last_change, &suspend) {
                    tracing::debug!("reaping site {name}");
                    mgr.queues.remove(&name);
                    drop(mgr);

                    queue.reinsert_ready_queue("reap").await;
                    crate::metrics_helper::remove_metrics_for_service(&format!(
                        "smtp_client:{name}"
                    ));
                    return Ok(());
                }
            } else if reap_deadline
                .as_ref()
                .map(|deadline| *deadline <= tokio::time::Instant::now())
                .unwrap_or(false)
                && !done_abort
            {
                let n = queue.abort_all_connections();
                tracing::warn!(
                    "{name}: {n} connections are outstanding, aborting them before reaping"
                );
                done_abort = true;
            } else if get_headroom() == 0 {
                queue.shrink_ready_queue_due_to_low_mem().await;
            } else if queue.activity.is_shutting_down() {
                let n = queue.connections.lock().len();
                tracing::debug!("{name}: waiting for {n} connections to close before reaping");
            }
        }
    }
}

pub type ReadyQueueHandle = Arc<ReadyQueue>;

pub struct ReadyQueue {
    name: String,
    queue_name_for_config_change_purposes_only: String,
    ready: Arc<Fifo>,
    mx: Option<Arc<MailExchanger>>,
    notify_maintainer: Arc<Notify>,
    notify_dispatcher: Arc<Notify>,
    connections: StdMutex<Vec<JoinHandle<()>>>,
    metrics: DeliveryMetrics,
    activity: Activity,
    consecutive_connection_failures: Arc<AtomicUsize>,
    path_config: ConfigHandle<EgressPathConfig>,
    queue_config: ConfigHandle<QueueConfig>,
    egress_pool: String,
    egress_source: EgressSource,
}

impl ReadyQueue {
    #[allow(unused)]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn insert(&self, msg: Message) -> Result<(), Message> {
        if low_memory() {
            msg.shrink().ok();
        }
        self.ready.push(msg)?;
        self.notify_maintainer.notify_one();
        self.notify_dispatcher.notify_one();

        Ok(())
    }

    pub fn ready_count(&self) -> usize {
        self.ready.len()
    }

    fn ideal_connection_count(&self, suspend: &Option<AdminSuspendReadyQEntryRef>) -> usize {
        if self.activity.is_shutting_down() {
            0
        } else if suspend.is_some() {
            0
        } else {
            let n = ideal_connection_count(
                self.ready_count(),
                self.path_config.borrow().connection_limit,
            );
            if n > 0 && get_headroom() == 0 {
                n.min(2)
            } else {
                n
            }
        }
    }

    #[instrument(skip(self))]
    async fn shrink_ready_queue_due_to_low_mem(&self) {
        let mut count = 0;
        let mut seen = 0;
        let mut requeue = 0;

        let mut reinsert = vec![];

        for msg in self.ready.drain() {
            seen += 1;
            if let Ok(true) = msg.shrink() {
                count += 1;
            }
            if let Err(msg) = self.ready.push(msg) {
                // The readyq is full and we can't reinsert; this
                // can happen when the system is busy and other
                // actors are adding more stuff to it.
                reinsert.push(msg);
                requeue += 1;
            }
        }

        if !reinsert.is_empty() {
            let activity = self.activity.clone();
            READYQ_RUNTIME
                .spawn("reinserting".to_string(), move || {
                    Ok(async move {
                        for msg in reinsert {
                            if let Err(err) = Dispatcher::reinsert_message(msg).await {
                                tracing::error!("error reinserting message: {err:#}");
                            }
                        }
                        drop(activity);
                    })
                })
                .await
                .expect("failed to spawn reinsertion");
        }

        tracing::error!(
            "did shrink {count} of out {seen} msgs in ready queue {} \
            due to memory shortage, and will requeue {requeue} \
            due to hitting constraints",
            self.name
        );
    }

    async fn reinsert_ready_queue(&self, reason: &str) {
        let msgs = self.ready.drain();
        if !msgs.is_empty() {
            let activity = self.activity.clone();
            READYQ_RUNTIME
                .spawn(
                    format!("reinserting {} due to {reason}", self.name),
                    move || {
                        Ok(async move {
                            for msg in msgs {
                                if let Err(err) = Dispatcher::reinsert_message(msg).await {
                                    tracing::error!("error reinserting message: {err:#}");
                                }
                            }
                            drop(activity);
                        })
                    },
                )
                .await
                .expect("failed to spawn reinsertion");
        }
    }

    fn abort_all_connections(&self) -> usize {
        let connections = self.connections.lock();
        for handle in connections.iter() {
            handle.abort();
        }
        connections.len()
    }

    async fn maintain(&self, suspend: &Option<AdminSuspendReadyQEntryRef>) {
        // Prune completed connection tasks and obtain the number of connections
        let current_connection_count = {
            let mut connections = self.connections.lock();
            connections.retain(|handle| !handle.is_finished());
            connections.len()
        };

        let path_config = self.path_config.borrow();

        tracing::trace!(
            "maintain {}: there are now {current_connection_count} connections, \
             suspended(admin)={}, queue_size={} (metrics.ready_count={})",
            self.name,
            suspend.is_some(),
            self.ready_count(),
            self.metrics.ready_count.get(),
        );

        if self.activity.is_shutting_down() {
            // We are shutting down; we want all messages to get saved.
            let msgs = self.ready.drain();
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

        if let Some(suspend) = suspend {
            let duration = suspend.get_duration();
            tracing::trace!(
                "{} is suspended until {duration:?}, throttling ready queue",
                self.name,
            );
            self.reinsert_ready_queue("suspend").await;
            self.notify_dispatcher.notify_waiters();
            return;
        }

        let ideal = self.ideal_connection_count(suspend);
        tracing::trace!(
            "maintain {}: computed ideal connection count as {ideal} \
            vs current {current_connection_count}",
            self.name
        );

        if current_connection_count < ideal {
            let limit = LimitSpec {
                limit: path_config.connection_limit,
                duration: path_config.client_timeouts.total_message_send_duration(),
            };

            for _ in current_connection_count..ideal {
                match limit.acquire_lease(&self.name).await {
                    Ok(lease) => {
                        // Open a new connection
                        let name = self.name.clone();
                        let queue_name_for_config_change_purposes_only =
                            self.queue_name_for_config_change_purposes_only.clone();
                        let mx = self.mx.clone();
                        let ready = Arc::clone(&self.ready);
                        let notify_dispatcher = self.notify_dispatcher.clone();
                        let path_config = self.path_config.clone();
                        let queue_config = self.queue_config.clone();
                        let metrics = self.metrics.clone();
                        let egress_source = self.egress_source.clone();
                        let egress_pool = self.egress_pool.clone();
                        let consecutive_connection_failures =
                            self.consecutive_connection_failures.clone();

                        tracing::trace!("spawning client for {name}");
                        if let Ok(handle) = READYQ_RUNTIME
                            .spawn(format!("smtp client {name}"), move || {
                                Ok(async move {
                                    if let Err(err) = Dispatcher::run(
                                        &name,
                                        queue_name_for_config_change_purposes_only,
                                        mx,
                                        ready,
                                        notify_dispatcher,
                                        queue_config,
                                        path_config,
                                        metrics,
                                        consecutive_connection_failures.clone(),
                                        egress_source,
                                        egress_pool,
                                        lease,
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
                            self.connections.lock().push(handle);
                        }
                    }
                    Err(err) => {
                        tracing::debug!(
                            "maintain {}: could not acquire connection lease: {err:#}",
                            self.name
                        );
                        break;
                    }
                }
            }
        }
    }

    fn reapable(
        &self,
        last_change: &Instant,
        suspend: &Option<AdminSuspendReadyQEntryRef>,
    ) -> bool {
        let ideal = self.ideal_connection_count(suspend);
        ideal == 0
            && self.connections.lock().is_empty()
            && ((last_change.elapsed() > Duration::from_secs(10 * 60))
                | self.activity.is_shutting_down())
            && self.ready_count() == 0
    }
}

impl Drop for ReadyQueue {
    fn drop(&mut self) {
        let n = self.ready_count();
        if n > 0 {
            tracing::error!("ReadyQueue::drop: {}: has {n} messages in queue", self.name);
        }
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
    /// You probably do not want to use queue_name_for_config_change_purposes_only
    /// in an SMTP Dispatcher, because it is a snapshot of the queue name of
    /// the very first scheduled queue to feed into the associated ReadyQueue.
    /// There may be many different scheduled queues feeding in, so if you
    /// want to resolve to the appropriate scheduled queue, you must do so
    /// via msg.get_queue_name() instead of using this stashed value.
    pub queue_name_for_config_change_purposes_only: String,
    pub ready: Arc<Fifo>,
    pub notify_dispatcher: Arc<Notify>,
    pub path_config: ConfigHandle<EgressPathConfig>,
    pub mx: Option<Arc<MailExchanger>>,
    pub metrics: DeliveryMetrics,
    pub shutting_down: ShutdownSubcription,
    pub activity: Activity,
    pub egress_source: EgressSource,
    pub egress_pool: String,
    pub delivered_this_connection: usize,
    pub msg: Option<Message>,
    pub delivery_protocol: String,
    pub suspended: Option<AdminSuspendReadyQEntryRef>,
    lease: LimitLease,
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        // Ensure that we re-queue any message that we had popped
        let msg = self.msg.take();
        let activity = self.activity.clone();
        let name = self.name.to_string();
        let notify_dispatcher = self.notify_dispatcher.clone();
        READYQ_RUNTIME
            .spawn_non_blocking("Dispatcher::drop".to_string(), move || {
                Ok(async move {
                    if let Some(msg) = msg {
                        if activity.is_shutting_down() {
                            Queue::save_if_needed_and_log(&msg).await;
                        } else {
                            if let Err(err) = Dispatcher::requeue_message(msg, false, None).await {
                                tracing::error!("error requeuing message: {err:#}");
                            }
                        }
                    } else {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        let ready_queue = ReadyQueueManager::get_by_name(&name);
                        if let Some(q) = ready_queue {
                            q.notify_maintainer.notify_one();
                        }
                        notify_dispatcher.notify_one();
                    }
                })
            })
            .ok();
    }
}

impl Dispatcher {
    #[instrument(skip(ready, metrics, notify_dispatcher))]
    async fn run(
        name: &str,
        queue_name_for_config_change_purposes_only: String,
        mx: Option<Arc<MailExchanger>>,
        ready: Arc<Fifo>,
        notify_dispatcher: Arc<Notify>,
        queue_config: ConfigHandle<QueueConfig>,
        path_config: ConfigHandle<EgressPathConfig>,
        metrics: DeliveryMetrics,
        consecutive_connection_failures: Arc<AtomicUsize>,
        egress_source: EgressSource,
        egress_pool: String,
        lease: LimitLease,
    ) -> anyhow::Result<()> {
        let activity = Activity::get(format!("ready_queue Dispatcher {name}"))?;

        let delivery_protocol = match &queue_config.borrow().protocol {
            DeliveryProto::Smtp { .. } => "ESMTP".to_string(),
            DeliveryProto::Lua { .. } => "Lua".to_string(),
            DeliveryProto::Maildir { .. } => "Maildir".to_string(),
        };

        let mut dispatcher = Self {
            name: name.to_string(),
            queue_name_for_config_change_purposes_only,
            ready,
            notify_dispatcher,
            mx,
            msg: None,
            path_config,
            metrics,
            shutting_down: ShutdownSubcription::get(),
            activity,
            egress_source,
            egress_pool,
            delivered_this_connection: 0,
            delivery_protocol,
            lease,
            suspended: None,
        };

        let mut queue_dispatcher: Box<dyn QueueDispatcher> = match &queue_config.borrow().protocol {
            DeliveryProto::Smtp { smtp } => {
                match SmtpDispatcher::init(&mut dispatcher, smtp).await? {
                    Some(disp) => Box::new(disp),
                    None => return Ok(()),
                }
            }
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

        // We get better throughput by being more aggressive with establishing
        // connections.
        if !dispatcher
            .path_config
            .borrow()
            .aggressive_connection_opening
        {
            dispatcher.obtain_message().await;
            if dispatcher.msg.is_none() {
                // We raced with another dispatcher and there is no
                // more work to be done; no need to open a new connection.
                dispatcher.lease.release().await;
                return Ok(());
            }
        }

        let mut connection_failures = vec![];

        loop {
            if !dispatcher.wait_for_message(&mut *queue_dispatcher).await? {
                // No more messages within our idle time; we can close
                // the connection
                tracing::debug!("{} Idling out connection", dispatcher.name);
                dispatcher.lease.release().await;
                queue_dispatcher.close_connection(&mut dispatcher).await?;
                return Ok(());
            }
            if dispatcher.activity.is_shutting_down() {
                tracing::debug!("{} shutting down", dispatcher.name);
                dispatcher.lease.release().await;
                queue_dispatcher.close_connection(&mut dispatcher).await?;
                return Ok(());
            }

            if let Err(err) = queue_dispatcher.attempt_connection(&mut dispatcher).await {
                connection_failures.push(format!("{err:#}"));
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
                            delivery_protocol: Some(&dispatcher.delivery_protocol),
                            tls_info: None,
                        })
                        .await;
                        Dispatcher::requeue_message(msg, true, None).await?;
                        dispatcher.metrics.inc_transfail();
                    }

                    if consecutive_connection_failures.fetch_add(1, Ordering::SeqCst)
                        > dispatcher
                            .path_config
                            .borrow()
                            .consecutive_connection_failures_before_delay
                    {
                        dispatcher.delay_ready_queue().await;
                    }
                    dispatcher.lease.release().await;
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
        if let Some(suspend) = AdminSuspendReadyQEntry::get_for_queue_name(&self.name) {
            // Do nothing here; wait_for_message will delay the ready queue,
            // and the regular cleanup will requeue self.msg
            tracing::trace!(
                "{} is suspended until {:?}",
                self.name,
                suspend.get_duration()
            );
            return Ok(());
        }

        // Process throttling before we acquire the Activity
        // guard, so that a delay due to throttling doesn't result
        // in a delay of shutdown
        let path_config = self.path_config.borrow();
        if let Some(throttle) = &path_config.max_message_rate {
            loop {
                let result = throttle
                    .throttle(format!("{}-message-rate", self.name))
                    .await
                    .context("apply max_message_rate throttle")?;

                if let Some(delay) = result.retry_after {
                    if delay >= path_config.client_timeouts.idle_timeout {
                        self.throttle_ready_queue(delay).await;
                        return Ok(());
                    }
                    tracing::trace!("{} throttled message rate, sleep for {delay:?}", self.name);
                    let mut shutdown = ShutdownSubcription::get();
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = shutdown.shutting_down() => {
                            return Ok(());
                        }
                    };
                } else {
                    break;
                }
            }
        }

        let msg = self.msg.as_ref().unwrap().clone();

        msg.load_meta_if_needed().await?;
        msg.load_data_if_needed().await?;

        let activity = match Activity::get_opt(format!(
            "ready_queue Dispatcher deliver_message {}",
            self.name
        )) {
            Some(a) => a,
            None => {
                anyhow::bail!("shutting down");
            }
        };

        self.delivered_this_connection += 1;

        if let Err(err) = queue_dispatcher.deliver_message(msg.clone(), self).await {
            // Transient failure; continue with another host
            tracing::debug!(
                "failed to send message id {:?} to {}: {err:#}",
                msg.id(),
                self.name,
            );
            return Err(err.into());
        }

        drop(activity);

        Ok(())
    }

    #[instrument(skip(msg))]
    pub async fn requeue_message(
        msg: Message,
        mut increment_attempts: bool,
        mut delay: Option<chrono::Duration>,
    ) -> anyhow::Result<()> {
        if !msg.is_meta_loaded() {
            msg.load_meta().await?;
        }
        let mut queue_name = msg.get_queue_name()?;

        // When increment_attempts is true, the intent is to handle a transient
        // failure for this message. In that circumstance we want to allow
        // the requeue_message event the opportunity to rebind the message
        // to an alternative scheduled queue.
        // Moving to another queue will make the message immediately eligible
        // for delivery in that new queue.
        if increment_attempts {
            match load_config().await {
                Ok(mut config) => {
                    let result: anyhow::Result<()> = config
                        .async_call_callback(&REQUEUE_MESSAGE_SIG, msg.clone())
                        .await;

                    match result {
                        Ok(()) => {
                            let queue_name_after = msg.get_queue_name()?;
                            if queue_name != queue_name_after {
                                // We want to avoid the normal due-time adjustment
                                // that would kick in when incrementing attempts
                                // in Queue::requeue_message, but we still want the
                                // number to be incremented.
                                msg.increment_num_attempts();
                                increment_attempts = false;

                                // Avoid adding jitter as part of the queue change
                                delay = Some(chrono::Duration::zero());
                                // and ensure that the message is due now
                                msg.set_due(None).await?;

                                // and use the new queue name
                                queue_name = queue_name_after;
                            }
                        }
                        Err(err) => {
                            tracing::error!(
                                "Error while calling requeue_message event: {err:#}. \
                                 will reuse current queue"
                            );
                        }
                    }
                }
                Err(err) => {
                    tracing::error!(
                        "ReadyQueue::requeue_message: error getting \
                         lua config in order to call requeue_message event: \
                         {err:#}, will reuse current queue"
                    );
                }
            }
        }

        let queue = QueueManager::resolve(&queue_name).await?;
        queue.requeue_message(msg, increment_attempts, delay).await
    }

    #[instrument(skip(msg))]
    pub async fn reinsert_message(msg: Message) -> anyhow::Result<()> {
        if !msg.is_meta_loaded() {
            msg.load_meta().await?;
        }
        let queue_name = msg.get_queue_name()?;
        let queue = QueueManager::resolve(&queue_name).await?;
        queue.insert(msg).await
    }

    /// Take the contents of the ready queue and reinsert them into
    /// the corresponding scheduled queue(s) for immediate reconsideration.
    /// This should cause the message(s) to be picked up by non-suspended
    /// paths to be delivered without additional delay.
    /// The insertion logic will take care of logging a transient failure
    /// if it transpires that no sources are enabled for the message.
    pub async fn reinsert_ready_queue(&mut self) {
        let mut msgs = self.ready.drain();
        if let Some(msg) = self.msg.take() {
            msgs.push(msg);
        }
        if !msgs.is_empty() {
            tracing::debug!(
                "suspend: reinserting ready queue {} - {} messages",
                self.name,
                msgs.len()
            );
            let activity = self.activity.clone();
            READYQ_RUNTIME
                .spawn("reinserting".to_string(), move || {
                    Ok(async move {
                        for msg in msgs {
                            if let Err(err) = Self::reinsert_message(msg).await {
                                tracing::error!("error reinserting message: {err:#}");
                            }
                        }
                        drop(activity);
                    })
                })
                .await
                .expect("failed to spawn reinsertion");
        }
    }

    pub async fn throttle_ready_queue(&mut self, delay: Duration) {
        let mut msgs = self.ready.drain();
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
                kumo_chrono_helper::MINUTE
            });
            READYQ_RUNTIME
                .spawn("requeue for throttle".to_string(), move || {
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
        let mut msgs = self.ready.drain();
        if let Some(msg) = self.msg.take() {
            msgs.push(msg);
        }
        if !msgs.is_empty() {
            let activity = self.activity.clone();
            let name = self.name.clone();
            let egress_pool = self.egress_pool.clone();
            let egress_source = self.egress_source.name.clone();
            if response.is_transient() {
                self.metrics.inc_transfail_by(msgs.len());
            } else {
                self.metrics.inc_fail_by(msgs.len());
            }
            READYQ_RUNTIME
                .spawn(
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
                                    delivery_protocol: None,
                                    tls_info: None,
                                })
                                .await;

                                if response.is_transient() {
                                    if let Err(err) =
                                        Self::requeue_message(msg, increment_attempts, None).await
                                    {
                                        tracing::error!("error requeuing message: {err:#}");
                                    }
                                } else if response.is_permanent() {
                                    SpoolManager::remove_from_spool(*msg.id()).await.ok();
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
            content: "bulk delay of ready queue: \
                too many successive connection failures \
                where there was no answer from any hosts listed in MX"
                .to_string(),
            command: None,
        })
        .await;
    }

    #[instrument(skip(self))]
    async fn obtain_message(&mut self) -> bool {
        if self.msg.is_some() {
            return true;
        }
        loop {
            self.msg = self.ready.pop();
            if let Some(msg) = &self.msg {
                if let Ok(queue_name) = msg.get_queue_name() {
                    if let Some(entry) = AdminBounceEntry::get_for_queue_name(&queue_name) {
                        let msg = self.msg.take().unwrap();
                        entry.log(msg.clone(), None).await;
                        SpoolManager::remove_from_spool(*msg.id()).await.ok();
                        continue;
                    }
                }

                return true;
            } else {
                return false;
            }
        }
    }

    fn get_suspension(&mut self) -> Option<AdminSuspendReadyQEntryRef> {
        if let Some(suspend) = &self.suspended {
            if !suspend.has_expired() {
                return Some(suspend.clone());
            }
        }

        if let Some(suspend) = AdminSuspendReadyQEntry::get_for_queue_name(&self.name) {
            self.suspended.replace(suspend);
        } else {
            self.suspended.take();
        }

        self.suspended.as_ref().cloned()
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

        if self.delivered_this_connection >= self.path_config.borrow().max_deliveries_per_connection
        {
            tracing::trace!(
                "Sent {} and limit is {}, close and make a new connection",
                self.delivered_this_connection,
                self.path_config.borrow().max_deliveries_per_connection,
            );
            let closed = queue_dispatcher.close_connection(self).await?;
            if closed {
                return Ok(false);
            }
        }

        if let Some(suspend) = self.get_suspension() {
            let duration = suspend.get_duration();
            tracing::trace!(
                "{} is suspended until {duration:?}, throttling ready queue",
                self.name,
            );
            self.reinsert_ready_queue().await;
            // Close the connection and stop trying to deliver
            return Ok(false);
        }

        if self
            .lease
            .extend(
                self.path_config
                    .borrow()
                    .client_timeouts
                    .total_message_send_duration(),
            )
            .await
            .is_err()
        {
            tracing::trace!(
                "{}: unable to extend lease, closing out this connection",
                self.name,
            );
            return Ok(false);
        }

        if self.obtain_message().await {
            return Ok(true);
        }

        let idle_timeout = self.path_config.borrow().client_timeouts.idle_timeout;
        let idle_deadline = tokio::time::Instant::now() + idle_timeout;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(idle_deadline) => {
                    return Ok(false);
                },
                _ = self.notify_dispatcher.notified() => {
                    if self.activity.is_shutting_down() {
                        return Ok(false);
                    }
                    if let Some(suspend) = self.get_suspension() {
                        let duration = suspend.get_duration();
                        tracing::trace!(
                            "{} is suspended until {duration:?}, throttling ready queue",
                            self.name,
                        );
                        self.reinsert_ready_queue().await;
                        // Close the connection and stop trying to deliver
                        return Ok(false);
                    }
                    if self.obtain_message().await {
                        return Ok(true);
                    }
                    // we raced with another dispatcher;
                    // snooze and try again
                    continue;
                }
                _ = self.shutting_down.shutting_down() => {
                    return Ok(false);
                }
            };
        }
    }
}

/// Use an exponential decay curve in the increasing form, asymptotic up to connection_limit,
/// passes through 0.0, increasing but bounded to connection_limit.
///
/// Visualize on wolframalpha: "plot 32 * (1-exp(-x * 0.023)), x from 0 to 100, y from 0 to 32"
pub fn ideal_connection_count(queue_size: usize, connection_limit: usize) -> usize {
    let factor = 0.023;
    let goal = (connection_limit as f32)
        * (1. - (-1.0 * queue_size as f32 * factor).exp()).min(queue_size as f32);
    goal.ceil().min(queue_size as f32) as usize
}

#[cfg(test)]
mod test {
    use super::*;

    fn compute_targets_for_limit(max_connections: usize) -> Vec<(usize, usize)> {
        let sizes = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 32, 64, 128, 256, 400, 512, 1024,
        ];
        sizes
            .iter()
            .map(|&queue_size| {
                (
                    queue_size,
                    ideal_connection_count(queue_size, max_connections),
                )
            })
            .collect()
    }

    #[test]
    fn connection_limit_32() {
        let targets = compute_targets_for_limit(32);
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
    #[test]

    fn connection_limit_1024() {
        let targets = compute_targets_for_limit(1024);
        assert_eq!(
            vec![
                (0, 0),
                (1, 1),
                (2, 2),
                (3, 3),
                (4, 4),
                (5, 5),
                (6, 6),
                (7, 7),
                (8, 8),
                (9, 9),
                (10, 10),
                (20, 20),
                (32, 32),
                (64, 64),
                (128, 128),
                (256, 256),
                (400, 400),
                (512, 512),
                (1024, 1024)
            ],
            targets
        );
    }
}
