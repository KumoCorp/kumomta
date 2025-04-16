use crate::delivery_metrics::{DeliveryMetrics, ReadyCountBundle};
use crate::egress_source::EgressSource;
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::http_server::admin_suspend_ready_q_v1::{
    AdminSuspendReadyQEntry, AdminSuspendReadyQEntryRef,
};
use crate::http_server::admin_suspend_v1::AdminSuspendEntry;
use crate::http_server::inject_v1::HttpInjectionGeneratorDispatcher;
use crate::http_server::queue_name_multi_index::CachedEntry;
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::lua_deliver::LuaQueueDispatcher;
use crate::metrics_helper::TOTAL_READYQ_RUNS;
use crate::queue::{
    DeliveryProto, IncrementAttempts, InsertContext, InsertReason, Queue, QueueConfig,
    QueueManager, QueueState,
};
use crate::smtp_dispatcher::{MxListEntry, OpportunisticInsecureTlsHandshakeError, SmtpDispatcher};
use crate::smtp_server::DeferredSmtpInjectionDispatcher;
use crate::spool::SpoolManager;
use anyhow::Context;
use arc_swap::ArcSwap;
use async_trait::async_trait;
use config::epoch::ConfigEpoch;
use config::{declare_event, load_config};
use dashmap::DashMap;
use dns_resolver::MailExchanger;
use kumo_api_types::egress_path::{
    ConfigRefreshStrategy, EgressPathConfig, MemoryReductionPolicy, WakeupStrategy,
};
use kumo_server_common::config_handle::ConfigHandle;
use kumo_server_lifecycle::{is_shutting_down, Activity, ShutdownSubcription};
use kumo_server_memory::{
    get_headroom, memory_status, subscribe_to_memory_status_changes_async, MemoryStatus,
};
use kumo_server_runtime::{get_named_runtime, spawn, Runtime};
use message::message::{MessageList, QueueNameComponents};
use message::Message;
use parking_lot::FairMutex;
use prometheus::Histogram;
use rfc5321::{EnhancedStatusCode, Response};
use serde::Serialize;
use std::fmt::Debug;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use throttle::limit::{LimitLease, LimitSpecWithDuration};
use throttle::ThrottleSpec;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::instrument;
use uuid::Uuid;

static MANAGER: LazyLock<ReadyQueueManager> = LazyLock::new(|| ReadyQueueManager::new());
static READYQ_RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("readyq", |cpus| cpus / 2, &READYQ_THREADS).unwrap());
static RQMAINT_THREADS: AtomicUsize = AtomicUsize::new(0);
static RQMAINT_RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("readyq_maint", |cpus| cpus / 4, &RQMAINT_THREADS).unwrap());

pub fn set_ready_qmaint_threads(n: usize) {
    RQMAINT_THREADS.store(n, Ordering::SeqCst);
}

declare_event! {
pub static GET_EGRESS_PATH_CONFIG_SIG: Single(
    "get_egress_path_config",
    routing_domain: String,
    egress_source: String,
    site_name: String
) -> EgressPathConfig;
}

static INSERT_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "ready_queue_insert_latency",
        "latency of ReadyQueue::insert operations",
    )
    .unwrap()
});

const ONE_MINUTE: Duration = Duration::from_secs(60);
const AGE_OUT_INTERVAL: Duration = Duration::from_secs(10 * 60);
static READYQ_THREADS: AtomicUsize = AtomicUsize::new(0);

pub fn set_readyq_threads(n: usize) {
    READYQ_THREADS.store(n, Ordering::SeqCst);
}

fn readyq_spawn<FUT, N: AsRef<str>>(
    path_config: &ConfigHandle<EgressPathConfig>,
    site_name: &str,
    name: N,
    fut: FUT,
) -> std::io::Result<JoinHandle<FUT::Output>>
where
    FUT: Future + Send + 'static,
    FUT::Output: Send,
{
    if let Some(name) = &path_config.borrow().readyq_pool_name {
        match get_named_runtime(name) {
            Some(runtime) => {
                return runtime.spawn(name, fut);
            }
            None => {
                tracing::error!(
                    "readyq {site_name} configured to use readyq_pool_name={name} \
                            but that thread pool is not defined",
                );
            }
        }
    }
    READYQ_RUNTIME.spawn(name, fut)
}

/// Represents a reserved place in a ready queue.
/// The reservation can be "redeemed" to infallibly
/// insert a Message into that queue, or dropped to
/// release the reservation.
pub struct FifoReservation {
    fifo: Arc<Fifo>,
    redeemed: bool,
}

impl Drop for FifoReservation {
    fn drop(&mut self) {
        if !self.redeemed {
            self.fifo.num_reserved.fetch_sub(1, Ordering::Relaxed);
            self.fifo.count.dec();
        }
    }
}

impl FifoReservation {
    /// Redeem the reservation and insert message into the associated Fifo
    pub fn redeem(mut self, message: Message) {
        self.redeemed = true;
        let mut list = self.fifo.list.lock();
        list.push_back(message);
        self.fifo.num_reserved.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct Fifo {
    list: FairMutex<MessageList>,
    count: ReadyCountBundle,
    capacity: AtomicUsize,
    num_reserved: AtomicUsize,
}

impl Fifo {
    pub fn new(capacity: usize, count: ReadyCountBundle) -> Self {
        Self {
            count,
            list: FairMutex::new(MessageList::new()),
            capacity: AtomicUsize::new(capacity),
            num_reserved: AtomicUsize::new(0),
        }
    }

    /// Attempt to reserve a place in the ready queue.
    /// The reservation can be used to subsequently infallibly
    /// insert into the fifo.
    pub fn reserve(self: Arc<Self>) -> Option<FifoReservation> {
        {
            let list = self.list.lock();
            if self.num_reserved.load(Ordering::Relaxed) + list.len() + 1
                > self.capacity.load(Ordering::Relaxed)
            {
                return None;
            }
            self.num_reserved.fetch_add(1, Ordering::Relaxed);
            self.count.inc();
        }
        Some(FifoReservation {
            fifo: self,
            redeemed: false,
        })
    }

    pub fn push(&self, msg: Message) -> Result<(), Message> {
        {
            let mut list = self.list.lock();
            if self.num_reserved.load(Ordering::Relaxed) + list.len() + 1
                > self.capacity.load(Ordering::Relaxed)
            {
                return Err(msg);
            }
            list.push_back(msg);
        }
        // Increment the count after we've dropped the mutexguard on the list
        self.count.inc();
        Ok(())
    }

    #[must_use]
    pub fn pop(&self) -> Option<Message> {
        let msg = self.list.lock().pop_front()?;
        self.count.dec();
        Some(msg)
    }

    #[must_use]
    pub fn take_list(&self) -> MessageList {
        let messages = self.list.lock().take();
        self.count.sub(messages.len());
        messages
    }

    /// Adjust the capacity of the Fifo.
    /// If the capacity is the same, nothing changes.
    /// Any messages that won't fit into the updated capacity are
    /// returned to the caller, who is responsible for re-inserting
    /// those messages into the scheduled queue
    #[must_use]
    pub fn update_capacity(&self, capacity: usize) -> MessageList {
        let mut excess = MessageList::new();

        if self.capacity.load(Ordering::Relaxed) == capacity {
            return excess;
        }

        let mut list = self.list.lock();
        let reserved = self.num_reserved.load(Ordering::Relaxed);
        self.capacity.store(capacity, Ordering::Relaxed);

        while reserved + list.len() > capacity {
            match list.pop_back() {
                Some(msg) => {
                    excess.push_back(msg);
                }
                None => {
                    break;
                }
            }
        }

        self.count.sub(excess.len());
        excess
    }

    pub fn len(&self) -> usize {
        let list = self.list.lock();
        list.len() + self.num_reserved.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug)]
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
    queues: DashMap<String, ReadyQueueHandle>,
}

impl ReadyQueueManager {
    pub fn new() -> Self {
        kumo_server_runtime::get_main_runtime().spawn(ReadyQueueManager::queue_config_maintainer());

        Self::default()
    }

    pub fn number_of_queues() -> usize {
        MANAGER.queues.len()
    }

    pub fn all_queues() -> Vec<ReadyQueueHandle> {
        let mut queues = vec![];
        for item in MANAGER.queues.iter() {
            queues.push(item.value().clone());
        }
        queues
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
                                mx_list.push(a.to_string());
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

        let path_config: EgressPathConfig = config
            .async_call_callback(
                &GET_EGRESS_PATH_CONFIG_SIG,
                (
                    routing_domain.to_string(),
                    egress_source.name.to_string(),
                    site_name.clone(),
                ),
            )
            .await
            .map_err(|err| {
                tracing::error!("Error while calling get_egress_path_config: {err:#}");
                err
            })?;
        config.put();

        Ok(ReadyQueueConfig {
            name,
            site_name,
            path_config,
            egress_source,
            mx,
        })
    }

    pub fn get_by_name(name: &str) -> Option<ReadyQueueHandle> {
        MANAGER.queues.get(name).map(|entry| entry.value().clone())
    }

    pub fn get_by_ready_queue_name(name: &ReadyQueueName) -> Option<ReadyQueueHandle> {
        Self::get_by_name(&name.name)
    }

    fn spawn_maintainer(name: &str, notify_maintainer: Arc<Notify>) -> anyhow::Result<()> {
        let name = name.to_string();
        RQMAINT_RUNTIME.spawn(
            format!("maintain {name}"),
            Self::maintainer_task(name, notify_maintainer),
        )?;
        Ok(())
    }

    pub async fn resolve_by_queue_name(
        queue_name: &str,
        queue_config: &ConfigHandle<QueueConfig>,
        egress_source: &str,
        egress_pool: &str,
        config_epoch: ConfigEpoch,
    ) -> anyhow::Result<ReadyQueueHandle> {
        let ReadyQueueConfig {
            name,
            site_name: _,
            path_config,
            egress_source,
            mx,
        } = Self::compute_config(queue_name, queue_config, egress_source).await?;

        let activity = Activity::get(format!("ReadyQueueHandle {name}"))?;

        let handle = MANAGER.queues.entry(name.clone()).or_insert_with(|| {
            let notify_maintainer = Arc::new(Notify::new());
            Self::spawn_maintainer(&name, notify_maintainer.clone())
                .expect("failed to spawn maintainer");
            let proto = queue_config.borrow().protocol.metrics_protocol_name();
            let service = format!("{proto}:{name}");
            let metrics = DeliveryMetrics::new(
                &service,
                &proto,
                egress_pool,
                &egress_source.name,
                &path_config.provider_name,
                mx.as_ref()
                    .map(|m| m.site_name.as_str())
                    .unwrap_or(queue_name),
            );
            let ready = Arc::new(Fifo::new(
                path_config.max_ready,
                metrics.ready_count.clone(),
            ));
            let notify_dispatcher = Arc::new(Notify::new());
            let next_config_refresh = Instant::now() + path_config.refresh_interval;

            Arc::new(ReadyQueue {
                name: name.clone(),
                queue_name_for_config_change_purposes_only: queue_name.to_string(),
                ready,
                mx,
                notify_dispatcher,
                notify_maintainer,
                connections: FairMutex::new(vec![]),
                num_connections: Arc::new(AtomicUsize::new(0)),
                path_config: ConfigHandle::new(path_config),
                queue_config: queue_config.clone(),
                egress_source,
                metrics,
                activity,
                consecutive_connection_failures: Arc::new(AtomicUsize::new(0)),
                egress_pool: egress_pool.to_string(),
                next_config_refresh: FairMutex::new(next_config_refresh),
                config_epoch: FairMutex::new(config_epoch),
                states: Arc::new(FairMutex::new(ReadyQueueStates::default())),
            })
        });
        Ok(handle.clone())
    }

    async fn queue_config_maintainer() {
        let mut shutdown = ShutdownSubcription::get();
        let mut epoch_subscriber = config::epoch::subscribe();
        let mut last_epoch = *epoch_subscriber.borrow_and_update();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    Self::check_config_refresh(&last_epoch, false).await;
                }
                _ = epoch_subscriber.changed() => {
                    let this_epoch = *epoch_subscriber.borrow_and_update();
                    tracing::debug!("queue_config_maintainer: epoch changed from \
                                     {last_epoch:?} -> {this_epoch:?}");
                    last_epoch = this_epoch;
                    Self::check_config_refresh(&last_epoch, true).await;
                }
                _ = shutdown.shutting_down() => {
                    tracing::info!("queue_config_maintainer stopping");
                    return;
                }
            }
        }
    }

    async fn check_config_refresh(epoch: &ConfigEpoch, epoch_changed: bool) {
        let now = Instant::now();

        tracing::debug!("check_config_refresh begins");
        let queues = ReadyQueueManager::all_queues();
        let mut num_due = 0;
        let num_queues = queues.len();

        for queue in queues {
            if is_shutting_down() {
                return;
            }
            if queue
                .perform_config_refresh_if_due(now, epoch, epoch_changed)
                .await
            {
                num_due += 1;
            }
        }

        tracing::debug!(
            "refreshed {num_due} configs out of {num_queues} ready queues in {:?}",
            now.elapsed()
        );
    }

    async fn maintainer_task(name: String, notify_maintainer: Arc<Notify>) -> anyhow::Result<()> {
        let mut shutdown = ShutdownSubcription::get();
        let mut memory = subscribe_to_memory_status_changes_async().await;
        let mut last_notify = Instant::now();
        let mut force_reap_deadline = None;
        let mut age_out_time = last_notify + AGE_OUT_INTERVAL;
        let mut done_abort = false;
        let mut shutting_down = false;

        let queue = ReadyQueueManager::get_by_name(&name).ok_or_else(|| {
            anyhow::anyhow!("ready_queue {name} not found when starting up maintainer_task")
        })?;

        loop {
            let wait_for_shutdown = async {
                if shutting_down {
                    tokio::time::sleep(Duration::from_secs(1)).await
                } else {
                    shutdown.shutting_down().await
                }
            };

            {
                let states = queue.states.lock();
                let limited = states.connection_limited.is_some()
                    || states.connection_rate_throttled.is_some();
                if limited {
                    // If we are throttled or up against a (possibly shared with
                    // other nodes in the cluster) connection limit, then arrange
                    // to wake up in about a minute, otherwise we wouldn't wake
                    // up until the next message is ready, or for 10 minutes,
                    // which is a bit too long.
                    // Ideally we'd get notified when someone releases a relevant
                    // lease, but we don't have a mechanism for that with redis
                    // at this time.
                    age_out_time = Instant::now() + ONE_MINUTE;
                }
            }

            tokio::select! {
                _ = wait_for_shutdown => {
                    shutting_down = true;
                    if force_reap_deadline.is_none() {
                        let path_config = queue.path_config.borrow();

                        let duration = path_config.system_shutdown_timeout.unwrap_or_else(|| {
                            path_config.client_timeouts.total_message_send_duration()
                        });
                        force_reap_deadline.replace(tokio::time::Instant::now() + duration);
                        tracing::debug!("{name}: reap deadline in {duration:?}");
                    }
                },
                _ = tokio::time::sleep_until(age_out_time.into()) => {
                    // Push it forward by a minute, so that we don't busy loop if we are
                    // not ready to age out now (still have open connections), and are not
                    // otherwise notified in a short time span
                    age_out_time = Instant::now() + ONE_MINUTE;
                },
                _ = memory.changed() => {
                    if get_headroom() == 0 {
                        queue.shrink_ready_queue_due_to_low_mem().await;
                    }
                },
                _ = notify_maintainer.notified() => {
                    last_notify = Instant::now();
                    age_out_time = last_notify + AGE_OUT_INTERVAL;
                },
            };

            TOTAL_READYQ_RUNS.inc();

            let suspend = AdminSuspendReadyQEntry::get_for_queue_name(&name);

            // Run the maintain method. It may block waiting to acquire
            // limit leases. Let's knock it out of that wait early
            // if we're shutting down or the memory status changes
            tokio::select! {
                _ = queue.maintain(&suspend) => {}
                _ = shutdown.shutting_down() => {
                    // Not doing all the same guff as the wait_for_shutdown block above
                    // because we will repeat that logic on the next loop iteration
                },
                _ = memory.changed() => {
                    // It's slightly undesirable to knock the maintain()
                    // method out of its acquire_lease wait if we transition
                    // from low->OK, but, we really do need to handle the
                    // other transition here.
                    if get_headroom() == 0 {
                        queue.shrink_ready_queue_due_to_low_mem().await;
                    }
                },
            };

            if queue.reapable(&last_notify, &suspend) {
                if MANAGER
                    .queues
                    .remove_if(&name, |_k, _q| queue.reapable(&last_notify, &suspend))
                    .is_some()
                {
                    tracing::debug!("reaping site {name}");

                    queue
                        .reinsert_ready_queue("reap", InsertReason::ReadyQueueWasReaped.into())
                        .await;
                    return Ok(());
                }
            } else if force_reap_deadline
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
            } else if queue.activity.is_shutting_down() {
                let n = queue.connections.lock().len();
                tracing::debug!("{name}: waiting for {n} connections to close before reaping");
            }
        }
    }
}

pub type ReadyQueueHandle = Arc<ReadyQueue>;

#[derive(Clone, Debug, Default, Serialize)]
pub struct ReadyQueueStates {
    pub connection_rate_throttled: Option<QueueState>,
    pub connection_limited: Option<QueueState>,
}

pub struct ReadyQueue {
    name: String,
    queue_name_for_config_change_purposes_only: String,
    ready: Arc<Fifo>,
    mx: Option<Arc<MailExchanger>>,
    notify_maintainer: Arc<Notify>,
    notify_dispatcher: Arc<Notify>,
    connections: FairMutex<Vec<JoinHandle<()>>>,
    num_connections: Arc<AtomicUsize>,
    metrics: DeliveryMetrics,
    activity: Activity,
    consecutive_connection_failures: Arc<AtomicUsize>,
    path_config: ConfigHandle<EgressPathConfig>,
    queue_config: ConfigHandle<QueueConfig>,
    egress_pool: String,
    egress_source: EgressSource,
    next_config_refresh: FairMutex<Instant>,
    config_epoch: FairMutex<ConfigEpoch>,
    pub states: Arc<FairMutex<ReadyQueueStates>>,
}

impl ReadyQueue {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn make_reservation(&self) -> Option<FifoReservation> {
        match self.ready.clone().reserve() {
            Some(res) => Some(res),
            None => {
                self.metrics.ready_full.inc();
                self.wakeup_dispatcher_or_maintainer();
                None
            }
        }
    }

    pub fn wakeup_all_dispatchers(&self) {
        self.notify_dispatcher.notify_waiters();
    }

    pub fn wakeup_dispatcher_or_maintainer(&self) {
        let path_config = self.path_config.borrow();
        let num_connections = self.num_connections.load(Ordering::SeqCst);

        match path_config.dispatcher_wakeup_strategy {
            WakeupStrategy::Aggressive => {
                self.notify_dispatcher.notify_waiters();
            }
            WakeupStrategy::Relaxed => {
                if num_connections > 0 {
                    self.notify_dispatcher.notify_one();
                }
            }
        }

        match path_config.maintainer_wakeup_strategy {
            WakeupStrategy::Aggressive => {
                self.notify_maintainer.notify_one();
            }
            WakeupStrategy::Relaxed => {
                let approx_conn_goal = ideal_connection_count(
                    self.ready.len(),
                    path_config.connection_limit.limit as usize,
                );
                if num_connections < approx_conn_goal {
                    self.notify_maintainer.notify_one();
                }
            }
        }
    }

    pub fn get_path_config(&self) -> &ConfigHandle<EgressPathConfig> {
        &self.path_config
    }

    pub async fn redeem_reservation(&self, msg: Message, reservation: FifoReservation) {
        let action = match memory_status() {
            MemoryStatus::LowMemory => self.path_config.borrow().low_memory_reduction_policy,
            MemoryStatus::NoMemory => self.path_config.borrow().no_memory_reduction_policy,
            MemoryStatus::Ok => MemoryReductionPolicy::NoShrink,
        };
        match action {
            MemoryReductionPolicy::NoShrink => {}
            MemoryReductionPolicy::ShrinkDataAndMeta => {
                msg.save_and_shrink().await.ok();
            }
            MemoryReductionPolicy::ShrinkData => {
                msg.save_and_shrink_data().await.ok();
            }
        }
        reservation.redeem(msg);
        self.wakeup_dispatcher_or_maintainer();
    }

    pub async fn insert(&self, msg: Message) -> Result<(), Message> {
        let _timer = INSERT_LATENCY.start_timer();
        let action = match memory_status() {
            MemoryStatus::LowMemory => self.path_config.borrow().low_memory_reduction_policy,
            MemoryStatus::NoMemory => self.path_config.borrow().no_memory_reduction_policy,
            MemoryStatus::Ok => MemoryReductionPolicy::NoShrink,
        };
        match action {
            MemoryReductionPolicy::NoShrink => {}
            MemoryReductionPolicy::ShrinkDataAndMeta => {
                msg.save_and_shrink().await.ok();
            }
            MemoryReductionPolicy::ShrinkData => {
                msg.save_and_shrink_data().await.ok();
            }
        }
        match self.ready.push(msg) {
            Ok(()) => {
                self.wakeup_dispatcher_or_maintainer();
                Ok(())
            }
            Err(msg) => {
                self.metrics.ready_full.inc();
                self.wakeup_dispatcher_or_maintainer();
                Err(msg)
            }
        }
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
                self.path_config.borrow().connection_limit.limit as usize,
            );
            if n > 0 && get_headroom() == 0 {
                n.min(2)
            } else {
                n
            }
        }
    }

    fn readyq_spawn<FUT, N: AsRef<str>>(
        &self,
        name: N,
        fut: FUT,
    ) -> std::io::Result<JoinHandle<FUT::Output>>
    where
        FUT: Future + Send + 'static,
        FUT::Output: Send,
    {
        readyq_spawn(&self.path_config, &self.name, name, fut)
    }

    #[instrument(skip(self))]
    async fn shrink_ready_queue_due_to_low_mem(&self) {
        let mut count = 0;
        let mut seen = 0;
        let mut requeue = 0;

        let mut reinsert = vec![];

        for msg in self.ready.take_list() {
            seen += 1;
            if let Ok(true) = msg.save_and_shrink().await {
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
            self.readyq_spawn("reinserting".to_string(), async move {
                for msg in reinsert {
                    if let Err(err) = Dispatcher::reinsert_message(
                        msg,
                        InsertReason::ReadyQueueWasDelayedDueToLowMemory.into(),
                    )
                    .await
                    {
                        tracing::error!("error reinserting message: {err:#}");
                    }
                }
                drop(activity);
            })
            .expect("failed to spawn reinsertion");
        }

        tracing::error!(
            "did shrink {count} of out {seen} msgs in ready queue {} \
            due to memory shortage, and will requeue {requeue} \
            due to hitting constraints",
            self.name
        );
    }

    async fn reinsert_ready_queue(&self, reason: &str, context: InsertContext) {
        let msgs = self.ready.take_list();
        if !msgs.is_empty() {
            let activity = self.activity.clone();
            self.readyq_spawn(
                format!("reinserting {} due to {reason}", self.name),
                async move {
                    for msg in msgs {
                        if let Err(err) = Dispatcher::reinsert_message(msg, context.clone()).await {
                            tracing::error!("error reinserting message: {err:#}");
                        }
                    }
                    drop(activity);
                },
            )
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
            self.metrics.ready_count.ready_count_by_service.get(),
        );

        if self.activity.is_shutting_down() {
            // We are shutting down; we want all messages to get saved.
            let msgs = self.ready.take_list();
            if !msgs.is_empty() {
                let activity = self.activity.clone();
                spawn(format!("saving messages for {}", self.name), async move {
                    for msg in msgs {
                        Queue::save_if_needed_and_log(&msg, None).await;
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
            self.reinsert_ready_queue("suspend", InsertReason::ReadyQueueWasSuspended.into())
                .await;
            self.wakeup_all_dispatchers();
            return;
        }

        let ideal = self.ideal_connection_count(suspend);
        tracing::trace!(
            "maintain {}: computed ideal connection count as {ideal} \
            vs current {current_connection_count}",
            self.name
        );

        if current_connection_count >= ideal {
            return;
        }

        let lease_duration = path_config
            .client_timeouts
            .total_connection_lifetime_duration();
        let limit_name = format!("kumomta.connection_limit.{}", self.name);
        let mut limits = vec![(
            &limit_name,
            LimitSpecWithDuration {
                spec: path_config.connection_limit,
                duration: lease_duration,
            },
        )];

        for (label, limit) in &path_config.additional_connection_limits {
            limits.push((
                label,
                LimitSpecWithDuration {
                    spec: *limit,
                    duration: lease_duration,
                },
            ));
        }
        // Check limits from smallest to largest so that we avoid
        // taking up a slot from a larger one only to hit a smaller
        // one and not do anything useful with the larger one
        limits.sort_by_key(|(_, LimitSpecWithDuration { spec, .. })| spec.limit);

        // Don't wait longer than the implied expiration of a lease
        let acquire_deadline = Instant::now() + lease_duration;

        'new_dispatcher: for _ in current_connection_count..ideal {
            let mut leases = vec![];
            for (label, limit) in &limits {
                self.states
                    .lock()
                    .connection_limited
                    .replace(QueueState::new(format!(
                        "acquiring connection lease {label}"
                    )));

                match limit.acquire_lease(label, acquire_deadline).await {
                    Ok(lease) => {
                        leases.push(lease);
                    }
                    Err(err @ throttle::Error::TooManyLeases(_)) => {
                        // Over budget; we'll try again later
                        tracing::debug!(
                            "maintain {}: could not acquire connection lease {label}: {err:#}",
                            self.name
                        );
                        self.states
                            .lock()
                            .connection_limited
                            .replace(QueueState::new(format!("TooManyLeases for {label}")));
                        break 'new_dispatcher;
                    }
                    Err(err) => {
                        // Some kind of error trying to acquire the lease, could be
                        // a redis/connectivity error, let's surface it
                        tracing::error!(
                            "maintain {}: could not acquire connection lease {label}: {err:#}",
                            self.name
                        );
                        self.states
                            .lock()
                            .connection_limited
                            .replace(QueueState::new(format!("Error acquiring {label}: {err:#}")));
                        break 'new_dispatcher;
                    }
                }
            }

            self.states.lock().connection_limited.take();

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
            let consecutive_connection_failures = self.consecutive_connection_failures.clone();
            let states = self.states.clone();
            let num_connections = self.num_connections.clone();

            tracing::trace!("spawning client for {name}");
            if let Ok(handle) = self.readyq_spawn(format!("smtp client {name}"), async move {
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
                    leases,
                    states,
                    num_connections,
                )
                .await
                {
                    tracing::debug!(
                        "Error in Dispatcher::run for {name}: {err:#} \
                         (consecutive_connection_failures={consecutive_connection_failures:?})"
                    );
                }
            }) {
                self.connections.lock().push(handle);
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
            && ((last_change.elapsed() >= AGE_OUT_INTERVAL) | self.activity.is_shutting_down())
            && self.ready_count() == 0
    }

    async fn perform_config_refresh_if_due(
        &self,
        now: Instant,
        epoch: &ConfigEpoch,
        epoch_changed: bool,
    ) -> bool {
        match self.path_config.borrow().refresh_strategy {
            ConfigRefreshStrategy::Ttl => {
                let due = *self.next_config_refresh.lock();
                if now >= due {
                    self.perform_config_refresh(epoch).await;
                    return true;
                }
                false
            }
            ConfigRefreshStrategy::Epoch => {
                if epoch_changed || *self.config_epoch.lock() != *epoch {
                    self.perform_config_refresh(epoch).await;
                    true
                } else {
                    false
                }
            }
        }
    }

    async fn perform_config_refresh(&self, epoch: &ConfigEpoch) {
        *self.config_epoch.lock() = *epoch;
        tracing::trace!("perform_config_refresh for {}", self.name);

        match ReadyQueueManager::compute_config(
            &self.queue_name_for_config_change_purposes_only,
            &self.queue_config,
            &self.egress_source.name,
        )
        .await
        {
            Ok(ReadyQueueConfig { path_config, .. }) => {
                if path_config != **self.path_config.borrow() {
                    let max_ready = path_config.max_ready;

                    let generation = self.path_config.update(path_config);
                    tracing::trace!(
                        "{}: refreshed get_egress_path_config to generation {generation}",
                        self.name
                    );
                    self.wakeup_all_dispatchers();
                    for msg in self.ready.update_capacity(max_ready) {
                        if let Err(err) = Dispatcher::reinsert_message(
                            msg,
                            InsertReason::MaxReadyWasReducedByConfigUpdate.into(),
                        )
                        .await
                        {
                            tracing::error!("error reinserting message: {err:#}");
                        }
                    }
                }
            }
            Err(err) => {
                tracing::error!("{}: refreshing get_egress_path_config: {err:#}", self.name);
            }
        }

        *self.next_config_refresh.lock() =
            Instant::now() + self.path_config.borrow().refresh_interval;
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

#[async_trait]
pub trait QueueDispatcher: Debug + Send {
    async fn deliver_message(
        &mut self,
        message: Vec<Message>,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()>;

    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()>;
    async fn have_more_connection_candidates(&mut self, dispatcher: &mut Dispatcher) -> bool;

    async fn close_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<bool>;

    /// If true, consider this session done with "success", just wind it down
    /// similar to how we behave if we've hit the max_deliveries_per_connection limit
    async fn has_terminated_ok(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        false
    }

    fn max_batch_size(&self) -> usize {
        1
    }

    fn min_batch_size(&self) -> usize {
        1
    }

    fn max_batch_latency(&self) -> Duration {
        Duration::from_secs(0)
    }
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
    pub activity: Activity,
    pub egress_source: EgressSource,
    pub egress_pool: String,
    pub delivered_this_connection: usize,
    pub msgs: Vec<Message>,
    pub delivery_protocol: String,
    pub suspended: Option<AdminSuspendReadyQEntryRef>,
    pub session_id: Uuid,
    leases: Vec<LimitLease>,
    batch_started: Option<tokio::time::Instant>,
    pub states: Arc<FairMutex<ReadyQueueStates>>,
    active_bounce: ArcSwap<Option<CachedEntry<AdminBounceEntry>>>,
    num_connections: Arc<AtomicUsize>,
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        // Ensure that we re-queue any message that we had popped
        let msgs = std::mem::take(&mut self.msgs);
        let activity = self.activity.clone();
        let name = self.name.to_string();
        self.num_connections.fetch_sub(1, Ordering::SeqCst);
        self.readyq_spawn("Dispatcher::drop".to_string(), async move {
            let had_msgs = !msgs.is_empty();

            for msg in msgs {
                if activity.is_shutting_down() {
                    Queue::save_if_needed_and_log(&msg, None).await;
                } else {
                    let response = Response {
                        code: 451,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 1,
                        }),
                        content: "KumoMTA internal: returning message to scheduled queue"
                            .to_string(),
                        command: None,
                    };

                    if let Err(err) = QueueManager::requeue_message(
                        msg,
                        IncrementAttempts::No,
                        None,
                        response,
                        InsertReason::DispatcherDrop.into(),
                    )
                    .await
                    {
                        tracing::error!("error requeuing message on Drop: {err:#}");
                    }
                }
            }

            if !had_msgs {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let ready_queue = ReadyQueueManager::get_by_name(&name);
                if let Some(q) = ready_queue {
                    q.wakeup_dispatcher_or_maintainer();
                }
            }
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
        leases: Vec<LimitLease>,
        states: Arc<FairMutex<ReadyQueueStates>>,
        num_connections: Arc<AtomicUsize>,
    ) -> anyhow::Result<()> {
        let activity = Activity::get(format!("ready_queue Dispatcher {name}"))?;

        let delivery_protocol = match &queue_config.borrow().protocol {
            DeliveryProto::Smtp { .. } => "ESMTP".to_string(),
            DeliveryProto::Lua { .. } => "Lua".to_string(),
            DeliveryProto::Maildir { .. } => "Maildir".to_string(),
            DeliveryProto::DeferredSmtpInjection => "DeferredSmtpInjection".to_string(),
            DeliveryProto::HttpInjectionGenerator => "HttpInjectionGenerator".to_string(),
            DeliveryProto::Null => {
                anyhow::bail!("Should not have a ready_queue for the null queue")
            }
        };

        let mut dispatcher = Self {
            name: name.to_string(),
            queue_name_for_config_change_purposes_only,
            ready,
            notify_dispatcher,
            mx,
            msgs: vec![],
            path_config,
            metrics,
            activity,
            egress_source,
            egress_pool,
            delivered_this_connection: 0,
            delivery_protocol,
            leases,
            suspended: None,
            batch_started: None,
            session_id: Uuid::new_v4(),
            states,
            active_bounce: Arc::new(None).into(),
            num_connections: num_connections.clone(),
        };
        dispatcher.num_connections.fetch_add(1, Ordering::SeqCst);

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
            DeliveryProto::HttpInjectionGenerator => {
                Box::new(HttpInjectionGeneratorDispatcher::new())
            }
            DeliveryProto::DeferredSmtpInjection => {
                Box::new(DeferredSmtpInjectionDispatcher::new())
            }
            DeliveryProto::Null => {
                anyhow::bail!("Should not have a ready_queue for the null queue")
            }
        };

        // We get better throughput by being more aggressive with establishing
        // connections.
        if !dispatcher
            .path_config
            .borrow()
            .aggressive_connection_opening
        {
            dispatcher.obtain_message(&mut *queue_dispatcher).await;
            if dispatcher.msgs.is_empty() {
                // We raced with another dispatcher and there is no
                // more work to be done; no need to open a new connection.
                dispatcher.release_leases().await;
                return Ok(());
            }
        }

        let mut connection_failures = vec![];
        let mut num_opportunistic_tls_failures = 0;
        let mut shutting_down = ShutdownSubcription::get();

        loop {
            if queue_dispatcher.has_terminated_ok(&mut dispatcher).await {
                tracing::debug!("{} connection session has terminated", dispatcher.name);
                dispatcher.release_leases().await;
                queue_dispatcher.close_connection(&mut dispatcher).await?;
                return Ok(());
            }

            if !dispatcher
                .wait_for_message(&mut *queue_dispatcher, &mut shutting_down)
                .await?
            {
                // No more messages within our idle time; we can close
                // the connection
                tracing::debug!("{} Idling out connection", dispatcher.name);
                dispatcher.release_leases().await;
                queue_dispatcher.close_connection(&mut dispatcher).await?;
                return Ok(());
            }

            let result = tokio::select! {
                _ = shutting_down.shutting_down() => {
                    tracing::debug!("{} shutting down", dispatcher.name);
                    dispatcher.release_leases().await;
                    queue_dispatcher.close_connection(&mut dispatcher).await?;
                    return Ok(());
                }
                result = queue_dispatcher.attempt_connection(&mut dispatcher) => {
                    result
                }
            };

            if let Err(err) = result {
                if OpportunisticInsecureTlsHandshakeError::is_match_anyhow(&err) {
                    num_opportunistic_tls_failures += 1;
                }
                connection_failures.push(format!("{err:#}"));
                if !queue_dispatcher
                    .have_more_connection_candidates(&mut dispatcher)
                    .await
                {
                    for msg in dispatcher.msgs.drain(..) {
                        let summary = if num_opportunistic_tls_failures == connection_failures.len()
                        {
                            "All failures are related to OpportunisticInsecure STARTTLS. \
                             Consider setting enable_tls=Disabled for this site. "
                        } else {
                            ""
                        };

                        let response = Response {
                            code: 400,
                            enhanced_code: None,
                            content: format!(
                                "KumoMTA internal: \
                                     failed to connect to any candidate \
                                     hosts: {summary}{}",
                                connection_failures.join(", ")
                            ),
                            command: None,
                        };

                        log_disposition(LogDisposition {
                            kind: RecordType::TransientFailure,
                            msg: msg.clone(),
                            site: &dispatcher.name,
                            peer_address: None,
                            response: response.clone(),
                            egress_pool: Some(&dispatcher.egress_pool),
                            egress_source: Some(&dispatcher.egress_source.name),
                            relay_disposition: None,
                            delivery_protocol: Some(&dispatcher.delivery_protocol),
                            tls_info: None,
                            source_address: None,
                            provider: dispatcher.path_config.borrow().provider_name.as_deref(),
                            session_id: Some(dispatcher.session_id),
                        })
                        .await;
                        QueueManager::requeue_message(
                            msg,
                            IncrementAttempts::Yes,
                            None,
                            response,
                            InsertReason::LoggedTransientFailure.into(),
                        )
                        .await?;
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
                    dispatcher.release_leases().await;
                    return Err(err);
                }
                tracing::debug!("{err:#}");
                // Try the next candidate MX address
                continue;
            }

            connection_failures.clear();
            consecutive_connection_failures.store(0, Ordering::SeqCst);

            let _timer_rollup = dispatcher.metrics.deliver_message_rollup.start_timer();
            if let Err(err) = dispatcher.deliver_message(&mut *queue_dispatcher).await {
                tracing::debug!("deliver_message: {err:#}");
                for msg in dispatcher.msgs.drain(..) {
                    let response = Response {
                        code: 400,
                        enhanced_code: None,
                        content: format!("KumoMTA internal: error in deliver_message: {err:#}"),
                        command: None,
                    };

                    log_disposition(LogDisposition {
                        kind: RecordType::TransientFailure,
                        msg: msg.clone(),
                        site: &dispatcher.name,
                        peer_address: None,
                        response: response.clone(),
                        egress_pool: Some(&dispatcher.egress_pool),
                        egress_source: Some(&dispatcher.egress_source.name),
                        relay_disposition: None,
                        delivery_protocol: Some(&dispatcher.delivery_protocol),
                        tls_info: None,
                        source_address: None,
                        provider: dispatcher.path_config.borrow().provider_name.as_deref(),
                        session_id: Some(dispatcher.session_id),
                    })
                    .await;
                    QueueManager::requeue_message(
                        msg,
                        IncrementAttempts::Yes,
                        None,
                        response,
                        InsertReason::LoggedTransientFailure.into(),
                    )
                    .await?;
                    dispatcher.metrics.inc_transfail();
                }
            }
        }
    }

    /// Returns true if we are throttled
    async fn check_throttle(
        &mut self,
        throttle: &ThrottleSpec,
        throttle_key: &str,
        throttle_label: &str,
        path_config: &EgressPathConfig,
    ) -> anyhow::Result<bool> {
        loop {
            let result = throttle
                .throttle(throttle_key)
                .await
                .with_context(|| format!("apply {throttle_label} throttle"))?;

            if let Some(delay) = result.retry_after {
                if delay >= path_config.client_timeouts.idle_timeout {
                    self.throttle_ready_queue(delay, InsertReason::MessageRateThrottle.into())
                        .await;
                    return Ok(true);
                }
                tracing::trace!("{} throttled message rate, sleep for {delay:?}", self.name);
                let mut shutdown = ShutdownSubcription::get();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = shutdown.shutting_down() => {
                        return Ok(true);
                    }
                };
            } else {
                return Ok(false);
            }
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
        let num_throttles = if path_config.max_message_rate.is_some() {
            1
        } else {
            0
        } + path_config.additional_message_rate_throttles.len();
        if num_throttles > 0 {
            let mut throttles = Vec::with_capacity(num_throttles);
            let message_rate_name;

            if let Some(throttle) = &path_config.max_message_rate {
                message_rate_name = format!("kumomta.max_message_rate.{}", self.name);
                throttles.push((&message_rate_name, throttle));
            }
            for (key, throttle) in &path_config.additional_message_rate_throttles {
                throttles.push((key, throttle));
            }

            // Check throttles from smallest to largest so that we avoid
            // taking up a slot from a larger one only to hit a smaller
            // one and not do anything useful with the larger one
            throttles.sort_by_key(|(_, spec)| {
                ((spec.limit as f64 / spec.period as f64) * 1_000_000.0) as u64
            });

            for (key, throttle) in throttles {
                if self
                    .check_throttle(&throttle, key, key, &path_config)
                    .await?
                {
                    return Ok(());
                }
            }
        }

        for msg in &self.msgs {
            msg.load_meta_if_needed().await?;
            msg.load_data_if_needed().await?;
        }

        let activity = match Activity::get_opt(format!(
            "ready_queue Dispatcher deliver_message {}",
            self.name
        )) {
            Some(a) => a,
            None => {
                anyhow::bail!("shutting down");
            }
        };

        self.delivered_this_connection += self.msgs.len();

        if let Err(err) = queue_dispatcher
            .deliver_message(self.msgs.clone(), self)
            .await
        {
            // Transient failure; continue with another host
            tracing::debug!("failed to send message batch to {}: {err:#}", self.name,);
            return Err(err);
        }

        drop(activity);

        Ok(())
    }

    #[instrument(skip(msg))]
    pub async fn reinsert_message(msg: Message, context: InsertContext) -> anyhow::Result<()> {
        if !msg.is_meta_loaded() {
            msg.load_meta().await?;
        }
        let queue_name = msg.get_queue_name()?;
        let queue = QueueManager::resolve(&queue_name).await?;
        queue.insert(msg, context, None).await
    }

    fn readyq_spawn<FUT, N: AsRef<str>>(
        &self,
        name: N,
        fut: FUT,
    ) -> std::io::Result<JoinHandle<FUT::Output>>
    where
        FUT: Future + Send + 'static,
        FUT::Output: Send,
    {
        readyq_spawn(&self.path_config, &self.name, name, fut)
    }

    /// Take the contents of the ready queue and reinsert them into
    /// the corresponding scheduled queue(s) for immediate reconsideration.
    /// This should cause the message(s) to be picked up by non-suspended
    /// paths to be delivered without additional delay.
    /// The insertion logic will take care of logging a transient failure
    /// if it transpires that no sources are enabled for the message.
    pub async fn reinsert_ready_queue(&mut self, context: InsertContext) {
        let mut msgs = self.ready.take_list();
        msgs.extend_from_iter(self.msgs.drain(..));
        if !msgs.is_empty() {
            tracing::debug!(
                "suspend: reinserting ready queue {} - {} messages",
                self.name,
                msgs.len()
            );
            let activity = self.activity.clone();
            self.readyq_spawn("reinserting".to_string(), async move {
                for msg in msgs {
                    if let Err(err) = Self::reinsert_message(msg, context.clone()).await {
                        if err.to_string() != "shutting down" {
                            tracing::error!("error reinserting message: {err:#}");
                        }
                    }
                }
                drop(activity);
            })
            .expect("failed to spawn reinsertion");
        }
    }

    pub async fn throttle_ready_queue(&mut self, delay: Duration, context: InsertContext) {
        let mut msgs = self.ready.take_list();
        msgs.extend_from_iter(self.msgs.drain(..));
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
            self.readyq_spawn("requeue for throttle".to_string(), async move {
                let response = Response {
                    code: 451,
                    enhanced_code: Some(EnhancedStatusCode {
                        class: 4,
                        subject: 4,
                        detail: 1,
                    }),
                    content: "KumoMTA internal: ready queue throttled".to_string(),
                    command: None,
                };
                for msg in msgs {
                    if let Err(err) = QueueManager::requeue_message(
                        msg,
                        IncrementAttempts::No,
                        Some(delay),
                        response.clone(),
                        context.clone(),
                    )
                    .await
                    {
                        tracing::error!("error requeuing message for throttle: {err:#}");
                    }
                }
                drop(activity);
            })
            .expect("failed to spawn requeue");
        }
    }

    #[instrument(skip(self))]
    pub async fn bulk_ready_queue_operation(&mut self, response: Response, context: InsertContext) {
        let mut msgs = self.ready.take_list();
        msgs.extend_from_iter(self.msgs.drain(..));
        if msgs.is_empty() {
            return;
        }
        if response.is_transient() {
            self.metrics.inc_transfail_by(msgs.len());
        } else {
            self.metrics.inc_fail_by(msgs.len());
        }
        let path_config = self.path_config.borrow();
        for msg in msgs {
            log_disposition(LogDisposition {
                kind: if response.is_transient() {
                    RecordType::TransientFailure
                } else {
                    RecordType::Bounce
                },
                msg: msg.clone(),
                site: &self.name,
                peer_address: None,
                response: response.clone(),
                egress_pool: Some(&self.egress_pool),
                egress_source: Some(&self.egress_source.name),
                relay_disposition: None,
                delivery_protocol: None,
                tls_info: None,
                source_address: None,
                provider: path_config.provider_name.as_deref(),
                session_id: Some(self.session_id),
            })
            .await;

            if response.is_transient() {
                if let Err(err) = QueueManager::requeue_message(
                    msg,
                    IncrementAttempts::Yes,
                    None,
                    response.clone(),
                    context.clone().add(InsertReason::LoggedTransientFailure),
                )
                .await
                {
                    tracing::error!("error requeuing for bulk {} operation: {err:#}", self.name);
                }
            } else if response.is_permanent() {
                SpoolManager::remove_from_spool(*msg.id()).await.ok();
            }
        }
    }

    #[instrument(skip(self))]
    async fn delay_ready_queue(&mut self) {
        tracing::debug!(
            "too many connection failures, delaying ready queue {}",
            self.name,
        );
        self.bulk_ready_queue_operation(
            Response {
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
            },
            InsertReason::TooManyConnectionFailures.into(),
        )
        .await;
    }

    /// Grab message(s) to satisfy the batching constraints.
    /// This function does not block.
    /// Returns true if the batch is ready to send on return.
    /// false indicates that it is not fully satisfied; we may
    /// have 0 or more messages accumulated.
    /// The batch latency is NOT considered by this function.
    #[instrument(skip(self))]
    async fn obtain_message(&mut self, queue_dispatcher: &mut dyn QueueDispatcher) -> bool {
        if self.msgs.len() >= queue_dispatcher.min_batch_size() {
            tracing::trace!(
                "already have {} messages which is >= min batch {}",
                self.msgs.len(),
                queue_dispatcher.min_batch_size()
            );
            self.batch_started.take();
            return true;
        }
        while self.msgs.len() < queue_dispatcher.max_batch_size() {
            if let Some(msg) = self.ready.pop() {
                if let Ok(queue_name) = msg.get_queue_name() {
                    if let Some(entry) = AdminBounceEntry::cached_get_for_queue_name(
                        &queue_name,
                        &self.active_bounce,
                    ) {
                        entry.log(msg.clone(), Some(&queue_name)).await;
                        SpoolManager::remove_from_spool(*msg.id()).await.ok();
                        continue;
                    }
                    if let Some(suspend) = AdminSuspendEntry::get_for_queue_name(&queue_name) {
                        let response = rfc5321::Response {
                            code: 451,
                            enhanced_code: Some(rfc5321::EnhancedStatusCode {
                                class: 4,
                                subject: 4,
                                detail: 4,
                            }),
                            content: format!(
                                "KumoMTA internal: scheduled queue is suspended: {}",
                                suspend.reason
                            ),
                            command: None,
                        };
                        log_disposition(LogDisposition {
                            kind: RecordType::TransientFailure,
                            msg: msg.clone(),
                            site: &self.name,
                            peer_address: None,
                            response: response.clone(),
                            egress_source: None,
                            egress_pool: None,
                            relay_disposition: None,
                            delivery_protocol: None,
                            provider: None,
                            tls_info: None,
                            source_address: None,
                            session_id: Some(self.session_id),
                        })
                        .await;

                        QueueManager::requeue_message(
                            msg,
                            IncrementAttempts::Yes,
                            None,
                            response,
                            InsertReason::LoggedTransientFailure.into(),
                        )
                        .await
                        .ok();
                        continue;
                    }
                }
                self.msgs.push(msg);
            } else {
                break;
            }
        }

        tracing::trace!(
            "now have {} messages. min batch {}, max {}",
            self.msgs.len(),
            queue_dispatcher.min_batch_size(),
            queue_dispatcher.max_batch_size()
        );

        if self.msgs.len() >= queue_dispatcher.min_batch_size() {
            // batch is satisfied and ready to go
            self.batch_started.take();
            true
        } else {
            // batch is not fully satisfied
            false
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

    /// Wait for sufficient message(s) to satisfy the batch constraints.
    /// This is bounded by the idle timeout and the maximum batch latency
    #[instrument(skip(self, shutting_down))]
    async fn wait_for_message(
        &mut self,
        queue_dispatcher: &mut dyn QueueDispatcher,
        shutting_down: &mut ShutdownSubcription,
    ) -> anyhow::Result<bool> {
        if self.activity.is_shutting_down() {
            for msg in self.msgs.drain(..) {
                Queue::save_if_needed_and_log(&msg, None).await;
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
            self.reinsert_ready_queue(InsertReason::ReadyQueueWasSuspended.into())
                .await;
            // Close the connection and stop trying to deliver
            return Ok(false);
        }

        for lease in &self.leases {
            if lease
                .extend(
                    self.path_config
                        .borrow()
                        .client_timeouts
                        .total_connection_lifetime_duration(),
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
        }

        if self.obtain_message(queue_dispatcher).await {
            return Ok(true);
        }
        let idle_timeout = self.path_config.borrow().client_timeouts.idle_timeout;
        let mut idle_deadline = tokio::time::Instant::now() + idle_timeout;

        tracing::trace!(
            "have {} messages. min batch {}, max {}, latency={:?}",
            self.msgs.len(),
            queue_dispatcher.min_batch_size(),
            queue_dispatcher.max_batch_size(),
            queue_dispatcher.max_batch_latency()
        );

        if !self.msgs.is_empty() {
            // If we have some messages and didn't return true from obtain_message
            // above, then we must have a partially fulfilled batch.
            // Ensure that we start tracking its latency for the deadline
            // calculation below, if we haven't already.
            if self.batch_started.is_none() {
                // We just started a batch
                self.batch_started.replace(tokio::time::Instant::now());
            }

            let batch_deadline = self.batch_started.expect("guaranteed to be set")
                + queue_dispatcher.max_batch_latency();

            // Use the smaller of the idle timeout and batch deadline
            // so that we don't timeout waiting for messages when we
            // have some already to go.
            // Note that we don't do anything here to recognize or
            // deal with a misconfiguration like the batch latency
            // being set higher than the maximum possible idle
            // timeout.
            idle_deadline = idle_deadline.min(batch_deadline);
        }

        loop {
            let notify_dispatcher = self.notify_dispatcher.clone();
            // Need to spawn this into a separate task otherwise the notifier future
            // isn't run in parallel
            let wait = tokio::spawn(async move {
                tokio::time::timeout_at(idle_deadline, notify_dispatcher.notified()).await
            });

            tokio::select! {
                result = wait => {
                    match result {
                        Ok(Err(_)) | Err(_) => {
                            // Timeout
                            self.batch_started.take();

                            // when the latency timer expires, we're satisfied by
                            // having any amount of messages in the batch
                            return Ok(!self.msgs.is_empty());
                        }
                        Ok(_) => {
                            if self.activity.is_shutting_down() {
                                return Ok(false);
                            }
                            if let Some(suspend) = self.get_suspension() {
                                let duration = suspend.get_duration();
                                tracing::trace!(
                                    "{} is suspended until {duration:?}, throttling ready queue",
                                    self.name,
                                );
                                self.reinsert_ready_queue(InsertReason::ReadyQueueWasSuspended.into()).await;
                                // Close the connection and stop trying to deliver
                                return Ok(false);
                            }
                            if self.obtain_message(queue_dispatcher).await {
                                return Ok(true);
                            }
                            // we raced with another dispatcher;
                            // snooze and try again
                            continue;
                        }
                    }
                }
                _ = shutting_down.shutting_down() => {
                    return Ok(false);
                }
            };
        }
    }

    async fn release_leases(&mut self) {
        for lease in &mut self.leases {
            lease.release().await;
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
