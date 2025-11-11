use crate::egress_source::{EgressPool, EgressPoolSourceSelector, SourceInsertResult};
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::http_server::admin_rebind_v1::AdminRebindEntry;
use crate::http_server::admin_suspend_v1::AdminSuspendEntry;
use crate::http_server::inject_v1::{make_generate_queue_config, GENERATOR_QUEUE_NAME};
use crate::http_server::queue_name_multi_index::CachedEntry;
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::config::QueueConfig;
use crate::queue::delivery_proto::DeliveryProto;
use crate::queue::insert_context::{InsertContext, InsertReason};
use crate::queue::maintainer::{maintain_named_queue, QMAINT_RUNTIME};
use crate::queue::manager::{QueueManager, MANAGER, SCHEDULED_QUEUE_COUNT};
use crate::queue::metrics::ScheduledMetrics;
use crate::queue::strategy::{QueueInsertResult, QueueStrategy, QueueStructure};
use crate::queue::{opt_timeout_at, IncrementAttempts, InsertResult, ReadyQueueFull};
use crate::ready_queue::ReadyQueueManager;
use crate::smtp_server::{make_deferred_queue_config, DEFERRED_QUEUE_NAME};
use crate::spool::SpoolManager;
use crate::xfer::request::AdminXferEntry;
use crate::xfer::{make_xfer_queue, SavedQueueInfo};
use anyhow::Context;
use arc_swap::ArcSwap;
use chrono::Utc;
use config::epoch::{get_current_epoch, ConfigEpoch};
use config::{declare_event, load_config, LuaConfig};
use humantime::format_duration;
use kumo_api_types::egress_path::{ConfigRefreshStrategy, MemoryReductionPolicy};
use kumo_api_types::xfer::XferProtocol;
use kumo_server_common::config_handle::ConfigHandle;
use kumo_server_lifecycle::{is_shutting_down, Activity, ShutdownSubcription};
use kumo_server_runtime::{get_main_runtime, spawn, spawn_blocking_on};
use kumo_template::TemplateEngine;
use message::queue_name::QueueNameComponents;
use message::Message;
use parking_lot::FairMutex;
use prometheus::IntGauge;
use rfc5321::{EnhancedStatusCode, Response};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};
use throttle::ThrottleResult;
use timeq::TimerEntryWithDelay;
use tokio::sync::Notify;
use tracing::instrument;
use uuid::Uuid;

pub type QueueHandle = Arc<Queue>;

pub struct Queue {
    pub name: Arc<String>,
    pub queue: QueueStructure,
    pub notify_maintainer: Arc<Notify>,
    last_change: FairMutex<Instant>,
    pub queue_config: ConfigHandle<QueueConfig>,
    metrics: OnceLock<ScheduledMetrics>,
    pub activity: Activity,
    source_selector: ArcSwap<EgressPoolSourceSelector>,
    next_config_refresh: FairMutex<Instant>,
    warned_strategy_change: AtomicBool,
    config_epoch: FairMutex<ConfigEpoch>,
    site_name: String,
    active_bounce: ArcSwap<Option<CachedEntry<AdminBounceEntry>>>,
}

impl Queue {
    pub async fn call_get_queue_config(
        name: &str,
        config: &mut LuaConfig,
    ) -> anyhow::Result<QueueConfig> {
        if name == GENERATOR_QUEUE_NAME {
            return make_generate_queue_config();
        }

        if name == DEFERRED_QUEUE_NAME {
            return make_deferred_queue_config();
        }

        if let Some(xfer) = make_xfer_queue(name) {
            return Ok(xfer);
        }

        if name == "null" {
            return Ok(QueueConfig {
                protocol: DeliveryProto::Null,
                retry_interval: Duration::from_secs(10),
                max_retry_interval: Some(Duration::from_secs(10)),
                ..QueueConfig::default()
            });
        }

        let components = QueueNameComponents::parse(&name);

        let queue_config: QueueConfig = config
            .async_call_callback(
                &GET_Q_CONFIG_SIG,
                (
                    components.domain,
                    components.tenant,
                    components.campaign,
                    components.routing_domain,
                ),
            )
            .await?;

        Ok(queue_config)
    }

    pub async fn new(name: String) -> anyhow::Result<QueueHandle> {
        let epoch = get_current_epoch();
        let mut config = load_config().await?;
        let queue_config = Self::call_get_queue_config(&name, &mut config).await?;

        let pool = EgressPool::resolve(queue_config.egress_pool.as_deref(), &mut config).await?;
        config.put();

        let source_selector = ArcSwap::new(EgressPoolSourceSelector::new(&pool).into());

        let activity = Activity::get(format!("Queue {name}"))?;
        let strategy = queue_config.strategy;
        let next_config_refresh = FairMutex::new(Instant::now() + queue_config.refresh_interval);

        let queue_config = ConfigHandle::new(queue_config);
        let site_name = match ReadyQueueManager::compute_queue_name(
            &name,
            &queue_config,
            "unspecified",
        )
        .await
        {
            Ok(ready_name) => ready_name.site_name,
            Err(err) => {
                // DNS resolution failed for whatever reason. We need to cook up a reasonable
                // site_name even though we cannot actually establish a connection or ready
                // queue for this domain.
                // We'll base it off the effective routing domain, but throw in a string to
                // help indicate at a glance that there is an issue with its DNS
                let reason = format!("{err:#}");
                let reason = if reason.contains("NXDOMAIN") {
                    "NXDOMAIN"
                } else {
                    // Any other DNS resolution failure
                    "DNSFAIL"
                };

                let components = QueueNameComponents::parse(&name);
                let routing_domain = components
                    .routing_domain
                    .as_deref()
                    .unwrap_or(&components.domain);

                format!("{reason}:{routing_domain}")
            }
        };

        let name = Arc::new(name);

        let handle = Arc::new(Queue {
            name: name.clone(),
            queue: QueueStructure::new(strategy),
            last_change: FairMutex::new(Instant::now()),
            queue_config,
            notify_maintainer: Arc::new(Notify::new()),
            metrics: OnceLock::new(),
            activity,
            source_selector,
            next_config_refresh,
            warned_strategy_change: AtomicBool::new(false),
            config_epoch: FairMutex::new(epoch),
            site_name,
            active_bounce: Arc::new(None).into(),
        });

        match strategy {
            QueueStrategy::SingletonTimerWheel | QueueStrategy::SingletonTimerWheelV2 => {
                // These use a global wheel maintainer
            }
            QueueStrategy::TimerWheel | QueueStrategy::SkipList => {
                Self::spawn_queue_maintainer(&handle)?;
            }
        }

        Ok(handle)
    }

    fn spawn_queue_maintainer(queue: &QueueHandle) -> anyhow::Result<()> {
        let queue = queue.clone();
        QMAINT_RUNTIME.spawn(format!("maintain {}", queue.name), async move {
            QMAINT_COUNT.inc();
            if let Err(err) = maintain_named_queue(&queue).await {
                tracing::error!("maintain_named_queue {}: {err:#}", queue.name);
            }
            QMAINT_COUNT.dec();
        })?;
        Ok(())
    }

    pub async fn queue_config_maintainer() {
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
        let names = QueueManager::all_queue_names();
        let mut num_due = 0;
        let mut num_reaped = 0;
        let num_queues = names.len();

        for name in names {
            if is_shutting_down() {
                return;
            }
            if let Some(queue) = QueueManager::get_opt(&name) {
                if queue.check_reap(now) {
                    num_reaped += 1;
                } else if queue
                    .perform_config_refresh_if_due(now, epoch, epoch_changed)
                    .await
                {
                    num_due += 1;
                }
            }
        }

        tracing::debug!(
            "refreshed {num_due} configs, reaped {num_reaped} \
             out of {num_queues} scheduled queues in {:?}",
            now.elapsed()
        );
    }

    fn check_reap(&self, now: Instant) -> bool {
        if !self.queue.is_empty() {
            return false;
        }

        let idle_at: Instant = *self.last_change.lock();
        let reap_after = self.queue_config.borrow().reap_interval;

        if now >= idle_at + reap_after {
            // NOT using QueueManager::remove here because we need to
            // be atomic wrt. another resolve operation

            if MANAGER
                .named
                .remove_if(self.name.as_str(), |_key, _q| self.queue.is_empty())
                .is_some()
            {
                tracing::debug!("idling out queue {}", self.name);
                SCHEDULED_QUEUE_COUNT.dec();
            }

            return true;
        }

        false
    }

    fn get_config_epoch(&self) -> ConfigEpoch {
        *self.config_epoch.lock()
    }

    fn set_config_epoch(&self, epoch: &ConfigEpoch) {
        *self.config_epoch.lock() = *epoch;
    }

    async fn perform_config_refresh_if_due(
        &self,
        now: Instant,
        epoch: &ConfigEpoch,
        epoch_changed: bool,
    ) -> bool {
        match self.queue_config.borrow().refresh_strategy {
            ConfigRefreshStrategy::Ttl => {
                let due = *self.next_config_refresh.lock();
                if now >= due {
                    self.perform_config_refresh(epoch).await;
                    return true;
                }

                false
            }
            ConfigRefreshStrategy::Epoch => {
                if epoch_changed || self.get_config_epoch() != *epoch {
                    self.perform_config_refresh(epoch).await;
                    true
                } else {
                    false
                }
            }
        }
    }

    async fn perform_config_refresh(&self, epoch: &ConfigEpoch) {
        if let Ok(mut config) = load_config().await {
            if let Ok(queue_config) = Queue::call_get_queue_config(&self.name, &mut config).await {
                match EgressPool::resolve(queue_config.egress_pool.as_deref(), &mut config).await {
                    Ok(pool) => {
                        if !self.source_selector.load().equivalent(&pool) {
                            self.source_selector
                                .store(EgressPoolSourceSelector::new(&pool).into());
                        }
                    }
                    Err(err) => {
                        tracing::error!(
                            "error while processing queue config update for {}: {err:#}",
                            self.name
                        );
                    }
                }

                let strategy = queue_config.strategy;

                self.queue_config.update(queue_config);
                self.set_config_epoch(epoch);

                if self.queue.strategy() != strategy
                    && !self.warned_strategy_change.load(Ordering::Relaxed)
                {
                    tracing::warn!(
                        "queue {} strategy change from {:?} to {:?} \
                                requires either the queue to be reaped, \
                                or a restart of kumod to take effect. \
                                This warning will be shown only once per scheduled queue.",
                        self.name,
                        self.queue.strategy(),
                        strategy
                    );
                    self.warned_strategy_change.store(true, Ordering::Relaxed);
                }
            }
        }
        *self.next_config_refresh.lock() =
            Instant::now() + self.queue_config.borrow().refresh_interval;
    }

    /// Insert into the timeq, and updates the counters.
    fn timeq_insert(self: &Arc<Self>, msg: Message) -> Result<(), Message> {
        tracing::trace!("timeq_insert {} due={:?}", self.name, msg.get_due());
        match self.queue.insert(msg, self) {
            QueueInsertResult::Inserted { should_notify } => {
                self.metrics().inc();
                if should_notify {
                    self.notify_maintainer.notify_one();
                }
                Ok(())
            }
            QueueInsertResult::Full(msg) => Err(msg),
        }
    }

    /// Removes all messages from the timeq, and updates the counters
    pub fn drain_timeq(&self) -> Vec<Message> {
        let msgs = self.queue.drain();
        if !msgs.is_empty() {
            self.metrics().sub(msgs.len());
            // Wake the maintainer so that it can see that the queue is
            // now empty and decide what it wants to do next.
            self.notify_maintainer.notify_one();
        }
        msgs
    }

    async fn do_rebind(
        self: &Arc<Self>,
        msg: Message,
        rebind: &Arc<AdminRebindEntry>,
        context: InsertContext,
    ) {
        async fn try_apply(msg: &Message, rebind: &Arc<AdminRebindEntry>) -> anyhow::Result<()> {
            if !msg.is_meta_loaded() {
                msg.load_meta().await?;
            }

            if rebind.request.trigger_rebind_event {
                let mut config = load_config().await?;
                config
                    .async_call_callback_non_default(
                        &REBIND_MESSAGE_SIG,
                        (msg.clone(), rebind.request.data.clone()),
                    )
                    .await
            } else {
                for (k, v) in &rebind.request.data {
                    msg.set_meta(k, v.clone())?;
                }
                Ok(())
            }
        }

        if let Err(err) = try_apply(&msg, rebind).await {
            tracing::error!("failed to apply rebind: {err:#}");
        }

        if msg.needs_save() {
            if let Err(err) = msg.save(None).await {
                tracing::error!("failed to save msg after rebind: {err:#}");
            }
        }

        let mut delay = None;

        let queue_name = match msg.get_queue_name() {
            Err(err) => {
                tracing::error!("failed to determine queue name for msg: {err:#}");
                if let Err(err) = self
                    .requeue_message_internal(
                        msg,
                        IncrementAttempts::No,
                        delay,
                        context.add(InsertReason::MessageGetQueueNameFailed),
                    )
                    .await
                {
                    tracing::error!(
                        "failed to requeue message to {} after failed rebind: {err:#}",
                        self.name
                    );
                }
                return;
            }
            Ok(name) => name,
        };

        let queue_holder;
        let queue = match QueueManager::resolve(&queue_name).await {
            Err(err) => {
                tracing::error!("failed to resolve queue `{queue_name}`: {err:#}");
                self
            }
            Ok(queue) => {
                queue_holder = queue;
                &queue_holder
            }
        };

        // If we changed queues, make the message immediately eligible for delivery
        if rebind.request.always_flush || queue.name != self.name {
            // Avoid adding jitter as part of the queue change
            delay = Some(chrono::Duration::zero());
            // and ensure that the message is due now
            msg.set_due(None).await.ok();
        }

        // If we changed queues, log an AdminRebind operation so that it is possible
        // to trace through the logs and understand what happened.
        if queue.name != self.name && !rebind.request.suppress_logging {
            log_disposition(LogDisposition {
                kind: RecordType::AdminRebind,
                msg: msg.clone(),
                site: "",
                peer_address: None,
                response: Response {
                    code: 250,
                    enhanced_code: None,
                    command: None,
                    content: format!(
                        "Rebound from {} to {queue_name}: {}",
                        self.name, rebind.request.reason
                    ),
                },
                egress_pool: None,
                egress_source: None,
                relay_disposition: None,
                delivery_protocol: None,
                tls_info: None,
                source_address: None,
                provider: None,
                session_id: None,
                recipient_list: None,
            })
            .await;
        }

        if let Err(err) = queue
            .requeue_message_internal(msg, IncrementAttempts::No, delay, context)
            .await
        {
            tracing::error!(
                "failed to requeue message to {} after failed rebind: {err:#}",
                queue.name
            );
        }
    }

    #[instrument(skip(self))]
    pub async fn rebind_all(self: &Arc<Self>, rebind: &Arc<AdminRebindEntry>) {
        let msgs = self.drain_timeq();
        let count = msgs.len();
        if count > 0 {
            for msg in msgs {
                self.do_rebind(msg, rebind, InsertReason::AdminRebind.into())
                    .await;
            }
        }
    }

    async fn do_xfer(
        self: &Arc<Self>,
        msg: Message,
        rebind: &Arc<AdminXferEntry>,
        context: InsertContext,
    ) {
        // Don't re-issue save_info if they xfer again to re-target
        // a queue that they already xfer'd previously
        if !XferProtocol::is_xfer_queue_name(&self.name) {
            if let Err(err) = SavedQueueInfo::save_info(&msg).await {
                tracing::error!("failed to apply xfer: {err:#}");
            }
        }

        let queue_name = rebind.request.protocol.to_queue_name();
        let queue_holder;
        let queue = match QueueManager::resolve(&queue_name).await {
            Err(err) => {
                tracing::error!("failed to resolve queue `{queue_name}`: {err:#}");
                self
            }
            Ok(queue) => {
                queue_holder = queue;
                &queue_holder
            }
        };

        if let Err(err) = msg.load_meta_if_needed().await {
            tracing::error!("failed to load meta: {err:#}");
        }
        if let Err(err) = msg.set_meta("queue", queue.name.to_string()) {
            tracing::error!("failed to save queue meta: {err:#}");
        }

        if msg.needs_save() {
            if let Err(err) = msg.save(None).await {
                tracing::error!("failed to save msg after rebind: {err:#}");
            }
        }

        if queue.name != self.name {
            log_disposition(LogDisposition {
                kind: RecordType::AdminRebind,
                msg: msg.clone(),
                site: "",
                peer_address: None,
                response: Response {
                    code: 250,
                    enhanced_code: None,
                    command: None,
                    content: format!("Rebound from {} to {queue_name}: {}", self.name, queue.name,),
                },
                egress_pool: None,
                egress_source: None,
                relay_disposition: None,
                delivery_protocol: None,
                tls_info: None,
                source_address: None,
                provider: None,
                session_id: None,
                recipient_list: None,
            })
            .await;
        }

        if let Err(err) = queue
            .requeue_message_internal(
                msg,
                IncrementAttempts::No,
                Some(chrono::Duration::seconds(0)),
                context,
            )
            .await
        {
            tracing::error!(
                "failed to requeue message to {} after failed rebind: {err:#}",
                queue.name
            );
        }
    }

    #[instrument(skip(self))]
    pub async fn xfer_all(self: &Arc<Self>, xfer: &Arc<AdminXferEntry>) {
        let msgs = self.drain_timeq();
        let count = msgs.len();
        if count > 0 {
            for msg in msgs {
                self.do_xfer(msg, xfer, InsertReason::AdminRebind.into())
                    .await;
            }
        }
    }

    async fn unwind_cancel_xfer(self: &Arc<Self>, msg: Message, context: InsertContext) {
        // Try to put it back!
        if let Err(err) = SavedQueueInfo::save_info(&msg).await {
            tracing::error!("failed to re-apply xfer: {err:#}. Giving up on message; cannot safely do anything more with it until restart");
            return;
        }

        if let Err(err) = self
            .requeue_message_internal(
                msg,
                IncrementAttempts::No,
                Some(chrono::Duration::seconds(0)),
                context,
            )
            .await
        {
            tracing::error!(
                "failed to requeue message to {} after failed xfer cancel: {err:#}",
                self.name
            );
        }
    }

    async fn undo_xfer(self: &Arc<Self>, msg: Message, context: InsertContext, reason: &str) {
        if let Err(err) = SavedQueueInfo::restore_info(&msg).await {
            tracing::error!("failed to cancel xfer: {err:#}");
        }

        if msg.needs_save() {
            if let Err(err) = msg.save(None).await {
                tracing::error!("failed to save msg after cancel xfer: {err:#}");
            }
        }

        // Put the message back into its originating queue
        let queue_name = match msg.get_queue_name() {
            Ok(name) => name,
            Err(err) => {
                tracing::error!(
                    "failed to get queue name from message after cancelling xfer: {err:#}"
                );
                return self.unwind_cancel_xfer(msg, context).await;
            }
        };

        let queue = match QueueManager::resolve(&queue_name).await {
            Err(err) => {
                tracing::error!(
                    "failed to resolve queue `{queue_name}` after cancelling xfer: {err:#}"
                );
                return self.unwind_cancel_xfer(msg, context).await;
            }
            Ok(queue) => queue,
        };

        log_disposition(LogDisposition {
            kind: RecordType::AdminRebind,
            msg: msg.clone(),
            site: "",
            peer_address: None,
            response: Response {
                code: 250,
                enhanced_code: None,
                command: None,
                content: format!("Rebound from {} to {queue_name}: {reason}", self.name),
            },
            egress_pool: None,
            egress_source: None,
            relay_disposition: None,
            delivery_protocol: None,
            tls_info: None,
            source_address: None,
            provider: None,
            session_id: None,
            recipient_list: None,
        })
        .await;

        if let Err(err) = queue
            .requeue_message_internal(
                msg,
                IncrementAttempts::No,
                Some(chrono::Duration::seconds(0)),
                context,
            )
            .await
        {
            tracing::error!(
                "failed to requeue message to {} after failed rebind: {err:#}",
                queue.name
            );
        }
    }

    #[instrument(skip(self))]
    pub async fn cancel_xfer_all(self: &Arc<Self>, reason: String) {
        if !XferProtocol::is_xfer_queue_name(&*self.name) {
            return;
        }

        let msgs = self.drain_timeq();
        let count = msgs.len();
        if count > 0 {
            for msg in msgs {
                self.undo_xfer(msg, InsertReason::AdminRebind.into(), &reason)
                    .await;
            }
        }
    }

    #[instrument(skip(self))]
    pub async fn bounce_all(&self, bounce: &AdminBounceEntry) {
        let msgs = self.drain_timeq();
        let count = msgs.len();
        if count > 0 {
            let name = self.name.clone();
            let bounce = bounce.clone();
            // Spawn the remove into a new task, to avoid holding the
            // mutable scope of self across a potentially very large
            // set of spool removal operations.  The downside is that the
            // reported numbers shown the to initial bounce request will
            // likely be lower, but it is better for the server to be
            // healthy than for that command to block and show 100% stats.
            let result =
                QMAINT_RUNTIME.spawn("bounce_all remove_from_spool".to_string(), async move {
                    for msg in msgs {
                        let id = *msg.id();
                        bounce.log(msg, Some(&name)).await;
                        SpoolManager::remove_from_spool(id).await.ok();
                    }
                });
            if let Err(err) = result {
                tracing::error!("Unable to schedule spool removal for {count} messages! {err:#}");
            }
        }
    }

    async fn increment_attempts_and_update_delay(
        &self,
        msg: Message,
    ) -> anyhow::Result<Option<Message>> {
        let id = *msg.id();
        // Pre-calculate the delay, prior to incrementing the number of attempts,
        // as the delay_for_attempt uses a zero-based attempt number to figure
        // the interval
        let num_attempts = msg.get_num_attempts();
        let delay = self.queue_config.borrow().delay_for_attempt(num_attempts);
        msg.increment_num_attempts();

        // Compute some jitter. The default retry_interval is 20 minutes for
        // which 1 minute is desired. To accomodate different intervals we translate
        // that to allowing up to 1/20th of the retry_interval as jitter, but we
        // cap it to 1 minute so that it doesn't result in excessive divergence
        // for very large intervals.
        let jitter_magnitude =
            (self.queue_config.borrow().retry_interval.as_secs_f32() / 20.0).min(60.0);
        let jitter = (rand::random::<f32>() * jitter_magnitude) - (jitter_magnitude / 2.0);
        let delay = kumo_chrono_helper::seconds(delay.num_seconds() + jitter as i64)?;

        match msg.get_scheduling().and_then(|sched| sched.expires) {
            Some(expires) => {
                // Per-message expiry
                match msg.delay_by(delay).await? {
                    Some(next_due) => {
                        if next_due >= expires {
                            tracing::debug!(
                                "expiring {id} {next_due} > scheduled expiry {expires}"
                            );
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
                                egress_pool: self.queue_config.borrow().egress_pool.as_deref(),
                                egress_source: None,
                                relay_disposition: None,
                                delivery_protocol: None,
                                tls_info: None,
                                source_address: None,
                                provider: self.queue_config.borrow().provider_name.as_deref(),
                                session_id: None,
                                recipient_list: None,
                            })
                            .await;
                            SpoolManager::remove_from_spool(id).await?;
                            return Ok(None);
                        }
                        tracing::trace!(
                            "increment_attempts_and_update_delay: delaying {id} \
                            by {delay} (num_attempts={num_attempts}), next_due={next_due:?}"
                        );
                    }
                    None => {
                        // Due immediately; cannot be an expiry.
                        // I really wouldn't expect to hit this ever; seems impossible!
                        tracing::trace!(
                            "increment_attempts_and_update_delay: delaying {id} \
                            by {delay} (num_attempts={num_attempts}), next_due=immediately"
                        );
                    }
                }
            }
            None => {
                // Regular queue based expiry

                let now = Utc::now();
                let max_age = self.queue_config.borrow().get_max_age();
                let age = msg.age(now);
                let delayed_age = age + delay;
                if delayed_age > max_age {
                    let delayed_age =
                        format_duration(delayed_age.to_std().unwrap_or(Duration::ZERO));
                    let max_age = format_duration(max_age.to_std().unwrap_or(Duration::ZERO));
                    tracing::debug!("expiring {id} {delayed_age} > {max_age}");
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
                                "Next delivery time would be {delayed_age} \
                        after creation, which exceeds max_age={max_age}"
                            ),
                            command: None,
                        },
                        egress_pool: self.queue_config.borrow().egress_pool.as_deref(),
                        egress_source: None,
                        relay_disposition: None,
                        delivery_protocol: None,
                        tls_info: None,
                        source_address: None,
                        provider: self.queue_config.borrow().provider_name.as_deref(),
                        session_id: None,
                        recipient_list: None,
                    })
                    .await;
                    SpoolManager::remove_from_spool(id).await?;
                    return Ok(None);
                }
                let next_due = msg.delay_by(delay).await?;
                tracing::trace!(
                    "increment_attempts_and_update_delay: delaying {id} \
                    by {delay} (num_attempts={num_attempts}), next_due={next_due:?}"
                );
            }
        }

        Ok(Some(msg))
    }

    /// Performs the raw re-insertion of a message into a scheduled queue.
    /// The requeue_message event is NOT called by this function.
    #[instrument(skip(self, msg))]
    pub async fn requeue_message_internal(
        self: &Arc<Self>,
        msg: Message,
        increment_attempts: IncrementAttempts,
        delay: Option<chrono::Duration>,
        context: InsertContext,
    ) -> anyhow::Result<()> {
        if increment_attempts == IncrementAttempts::Yes {
            match self.increment_attempts_and_update_delay(msg).await? {
                Some(msg) => {
                    return self.insert(msg, context, None).await;
                }
                None => {
                    // It was expired and removed from the spool
                    return Ok(());
                }
            };
        } else if let Some(delay) = delay {
            msg.delay_by(delay).await?;
        } else {
            msg.delay_with_jitter(60).await?;
        }

        if let Some(due) = msg.get_due() {
            let max_age = self.queue_config.borrow().get_max_age();
            // The age of the message at its next due time
            let due_age = msg.age(due);
            if due_age >= max_age {
                let id = *msg.id();
                tracing::debug!("expiring {id} {due_age} > {max_age}");
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
                        content: format!("Next delivery time {due_age} > {max_age}"),
                        command: None,
                    },
                    egress_pool: self.queue_config.borrow().egress_pool.as_deref(),
                    egress_source: None,
                    relay_disposition: None,
                    delivery_protocol: None,
                    tls_info: None,
                    source_address: None,
                    provider: self.queue_config.borrow().provider_name.as_deref(),
                    session_id: None,
                    recipient_list: None,
                })
                .await;
                SpoolManager::remove_from_spool(id).await?;
                return Ok(());
            }
        }

        self.insert(msg, context, None).await?;

        Ok(())
    }

    #[instrument(skip(self, msg))]
    async fn insert_delayed(
        self: &Arc<Self>,
        msg: Message,
        context: InsertContext,
    ) -> anyhow::Result<InsertResult> {
        tracing::trace!("insert_delayed {}", msg.id());

        match msg.get_due() {
            None => Ok(InsertResult::Ready(msg)),
            Some(due) => {
                let now = Utc::now();
                if due <= now {
                    Ok(InsertResult::Ready(msg))
                } else {
                    tracing::trace!("insert_delayed, locking timeq {}", msg.id());

                    match self.timeq_insert(msg.clone()) {
                        Ok(_) => {
                            if let Err(err) = self.did_insert_delayed(msg.clone(), context).await {
                                tracing::error!("while shrinking: {}: {err:#}", msg.id());
                            }
                            Ok(InsertResult::Delayed)
                        }
                        Err(msg) => Ok(InsertResult::Ready(msg)),
                    }
                }
            }
        }
    }

    #[instrument(skip(self, msg))]
    async fn force_into_delayed(
        self: &Arc<Self>,
        msg: Message,
        context: InsertContext,
    ) -> anyhow::Result<()> {
        tracing::trace!("force_into_delayed {}", msg.id());
        loop {
            match self.insert_delayed(msg.clone(), context.clone()).await? {
                InsertResult::Delayed => return Ok(()),
                // Maybe delay_with_jitter computed an immediate
                // time? Let's try again
                InsertResult::Ready(_) => {
                    msg.delay_with_jitter(60).await?;
                    continue;
                }
            }
        }
    }

    #[instrument(skip(msg))]
    pub async fn save_if_needed(
        msg: &Message,
        queue_config: Option<&ConfigHandle<QueueConfig>>,
    ) -> anyhow::Result<()> {
        tracing::trace!("save_if_needed {}", msg.id());
        if msg.needs_save() {
            msg.save(None).await?;
        }

        match queue_config {
            None => {
                // By convention, we are shutting down and all we needed to do
                // was the save
            }
            Some(queue_config) => {
                let config = queue_config.borrow();
                if config.shrink_policy.is_empty() {
                    msg.shrink()?;
                } else {
                    let interval = msg.delay();

                    let mut policy = MemoryReductionPolicy::ShrinkDataAndMeta;

                    for entry in config.shrink_policy.iter() {
                        if interval >= entry.interval {
                            policy = entry.policy;
                        }
                    }

                    match policy {
                        MemoryReductionPolicy::ShrinkDataAndMeta => {
                            msg.shrink()?;
                        }
                        MemoryReductionPolicy::ShrinkData => {
                            msg.shrink_data()?;
                        }
                        MemoryReductionPolicy::NoShrink => {}
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn save_if_needed_and_log(
        msg: &Message,
        queue_config: Option<&ConfigHandle<QueueConfig>>,
    ) {
        if let Err(err) = Self::save_if_needed(msg, queue_config).await {
            let id = msg.id();
            tracing::error!("error saving {id}: {err:#}");
        }
    }

    async fn did_insert_delayed(
        &self,
        msg: Message,
        mut context: InsertContext,
    ) -> anyhow::Result<()> {
        // Don't log Enumerated because we'll log 1 record for every message
        // in the spool, which doesn't seem useful.
        // We don't log records where LoggedTransientFailure is set, because
        // we already have a TransientFailure log record for those explaining
        // what happened.
        let log_delay = !context.only(InsertReason::Enumerated)
            && !context.contains(InsertReason::LoggedTransientFailure);

        if log_delay {
            if context.only(InsertReason::Received) && msg.get_scheduling().is_some() {
                context.note(InsertReason::ScheduledForLater);
            }

            let now = Utc::now();
            let due = msg.get_due().unwrap_or(now);
            let due_in = (due - now).to_std().unwrap_or(Duration::ZERO);

            log_disposition(LogDisposition {
                kind: RecordType::Delayed,
                msg: msg.clone(),
                site: "",
                peer_address: None,
                response: Response {
                    code: 400,
                    enhanced_code: None,
                    command: None,
                    content: format!(
                        "Context: {context}. Next due in {} at {due:?}",
                        format_duration(due_in)
                    ),
                },
                egress_source: None,
                egress_pool: None,
                relay_disposition: None,
                delivery_protocol: None,
                tls_info: None,
                source_address: None,
                provider: None,
                session_id: None,
                recipient_list: None,
            })
            .await;
        }

        Self::save_if_needed(&msg, Some(&self.queue_config)).await
    }

    async fn check_message_rate_throttle(&self) -> anyhow::Result<Option<ThrottleResult>> {
        if let Some(throttle) = &self.queue_config.borrow().max_message_rate {
            let result =
                Box::pin(throttle.throttle(format!("schedq-{}-message-rate", self.name))).await?;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    pub fn metrics(&self) -> &ScheduledMetrics {
        self.metrics.get_or_init(|| {
            let queue_config = self.queue_config.borrow();
            ScheduledMetrics::new(
                self.name.clone(),
                queue_config.egress_pool.as_deref().unwrap_or("unspecified"),
                &self.site_name,
                &queue_config.provider_name,
            )
        })
    }

    #[instrument(skip(self, msg))]
    pub async fn insert_ready(
        self: &Arc<Self>,
        msg: Message,
        mut context: InsertContext,
        deadline: Option<Instant>,
    ) -> anyhow::Result<()> {
        if let Some(b) =
            AdminBounceEntry::cached_get_for_queue_name(&self.name, &self.active_bounce)
        {
            let id = *msg.id();
            b.log(msg, Some(&self.name)).await;
            SpoolManager::remove_from_spool(id).await.ok();
            return Ok(());
        }

        // Don't promote to ready queue while suspended
        if let Some(suspend) = AdminSuspendEntry::get_for_queue_name(&self.name) {
            let remaining = suspend.get_duration();
            tracing::trace!("{} is suspended, delay={remaining:?}", self.name);

            let response = Response {
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
                session_id: None,
                recipient_list: None,
            })
            .await;

            Box::pin(QueueManager::requeue_message(
                msg,
                IncrementAttempts::Yes,
                None,
                response,
                InsertReason::LoggedTransientFailure.into(),
            ))
            .await?;

            return Ok(());
        }

        if let Some(result) = opt_timeout_at(deadline, self.check_message_rate_throttle()).await? {
            if let Some(delay) = result.retry_after {
                tracing::trace!("{} throttled message rate, delay={delay:?}", self.name);
                // We're not using jitter here because the throttle should
                // ideally result in smooth message flow and the jitter will
                // (intentionally) perturb that.
                let delay = chrono::Duration::from_std(delay).unwrap_or(kumo_chrono_helper::MINUTE);

                Box::pin(QueueManager::requeue_message(
                    msg,
                    IncrementAttempts::No,
                    Some(delay),
                    Response {
                        code: 451,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 4,
                        }),
                        content: format!(
                            "KumoMTA internal: {} throttled message rate, delay={delay:?}",
                            self.name
                        ),
                        command: None,
                    },
                    context.add(InsertReason::MessageRateThrottle),
                ))
                .await?;

                self.metrics().delay_due_to_message_rate_throttle().inc();

                return Ok(());
            }
        }

        opt_timeout_at(deadline, async {
            let mut config = load_config().await?;
            config
                .async_call_callback(&THROTTLE_INSERT_READY_SIG, msg.clone())
                .await?;
            config.put();
            Ok(())
        })
        .await?;

        if let Some(due) = msg.get_due() {
            let now = Utc::now();
            if due > now {
                tracing::trace!(
                    "{}: throttle_insert_ready_queue event throttled message rate, due={due:?}",
                    self.name
                );
                self.metrics().delay_due_to_throttle_insert_ready().inc();

                Box::pin(QueueManager::requeue_message(
                    msg,
                    IncrementAttempts::No,
                    None,
                    Response {
                        code: 451,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 4,
                        }),
                        content: format!(
                            "KumoMTA internal: {} throttle_insert_ready_queue event throttled message rate, due={due:?}",
                            self.name
                        ),
                        command: None,
                    },
                    context.add(InsertReason::ThrottledByThrottleInsertReadyQueue),
                ))
                .await?;
                return Ok(());
            }
        }

        if let Err(err) = self
            .insert_ready_impl(msg.clone(), &mut context, deadline)
            .await
        {
            tracing::debug!("insert_ready: {err:#}");

            if err.downcast_ref::<ReadyQueueFull>().is_none() {
                // It was a legit error while trying to do something useful

                Box::pin(QueueManager::requeue_message(
                    msg,
                    IncrementAttempts::Yes,
                    None,
                    Response {
                        code: 451,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 4,
                        }),
                        content: format!(
                            "KumoMTA internal: {} error while inserting into ready queue: {err:#}",
                            self.name
                        ),
                        command: None,
                    },
                    context.add(InsertReason::FailedToInsertIntoReadyQueue),
                ))
                .await?;
            } else {
                // Queue is full; try again shortly
                self.metrics().delay_due_to_ready_queue_full().inc();
                self.force_into_delayed(msg, context.add(InsertReason::ReadyQueueWasFull))
                    .await
                    .context("force_into_delayed")?;
            }
        }
        Ok(())
    }

    #[instrument(skip(self, msg))]
    async fn insert_ready_impl(
        &self,
        msg: Message,
        context: &mut InsertContext,
        deadline: Option<Instant>,
    ) -> anyhow::Result<()> {
        tracing::trace!("insert_ready {}", msg.id());

        match &self.queue_config.borrow().protocol {
            DeliveryProto::Smtp { .. }
            | DeliveryProto::Lua { .. }
            | DeliveryProto::Xfer { .. }
            | DeliveryProto::HttpInjectionGenerator => {
                let source_selector = self.source_selector.load();
                match source_selector
                    .select_and_insert(
                        &self.name,
                        &self.queue_config,
                        msg.clone(),
                        self.get_config_epoch(),
                        deadline,
                    )
                    .await?
                {
                    SourceInsertResult::Inserted => Ok(()),
                    SourceInsertResult::Delay(_duration) => {
                        log_disposition(LogDisposition {
                            kind: RecordType::TransientFailure,
                            msg,
                            site: "",
                            peer_address: None,
                            response: Response {
                                code: 451,
                                enhanced_code: Some(EnhancedStatusCode {
                                    class: 4,
                                    subject: 4,
                                    detail: 4,
                                }),
                                content: format!(
                                    "KumoMTA internal: no sources for {} pool=`{}` are eligible for selection at this time",
                                    self.name, source_selector.name
                                ),
                                command: None,
                            },
                            egress_pool: Some(&source_selector.name),
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: None,
                            tls_info: None,
                            source_address: None,
                            provider: self.queue_config.borrow().provider_name.as_deref(),
                            session_id: None,
                            recipient_list: None,
                        })
                        .await;
                        context.note(InsertReason::LoggedTransientFailure);
                        anyhow::bail!(
                            "no sources for {} pool=`{}` are eligible for selection at this time",
                            self.name,
                            source_selector.name
                        );
                    }
                    SourceInsertResult::NoSources => {
                        log_disposition(LogDisposition {
                            kind: RecordType::TransientFailure,
                            msg,
                            site: "",
                            peer_address: None,
                            response: Response {
                                code: 451,
                                enhanced_code: Some(EnhancedStatusCode {
                                    class: 4,
                                    subject: 4,
                                    detail: 4,
                                }),
                                content: format!(
                                    "KumoMTA internal: no sources available for {} pool=`{}`",
                                    self.name, source_selector.name,
                                ),
                                command: None,
                            },
                            egress_pool: Some(&source_selector.name),
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: None,
                            tls_info: None,
                            source_address: None,
                            provider: self.queue_config.borrow().provider_name.as_deref(),
                            session_id: None,
                            recipient_list: None,
                        })
                        .await;
                        context.note(InsertReason::LoggedTransientFailure);
                        anyhow::bail!(
                            "no sources available for {} pool=`{}`",
                            self.name,
                            source_selector.name
                        );
                    }
                    SourceInsertResult::FailedResolve(err) => {
                        log_disposition(LogDisposition {
                            kind: RecordType::TransientFailure,
                            msg,
                            site: "",
                            peer_address: None,
                            response: Response {
                                code: 451,
                                enhanced_code: Some(EnhancedStatusCode {
                                    class: 4,
                                    subject: 4,
                                    detail: 4,
                                }),
                                content: format!("failed to resolve queue {}: {err:#}", self.name),
                                command: None,
                            },
                            egress_pool: None,
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: None,
                            tls_info: None,
                            source_address: None,
                            provider: self.queue_config.borrow().provider_name.as_deref(),
                            session_id: None,
                            recipient_list: None,
                        })
                        .await;
                        context.note(InsertReason::LoggedTransientFailure);
                        anyhow::bail!("failed to resolve queue {}: {err:#}", self.name);
                    }
                }
            }
            DeliveryProto::DeferredSmtpInjection => {
                if let Some(site) = ReadyQueueManager::get_by_name(
                    "unspecified->deferred_smtp_inject.kumomta.internal@defersmtpinject",
                ) {
                    return site.insert(msg).await.map_err(|_| ReadyQueueFull.into());
                }

                let egress_source = "unspecified";
                let egress_pool = "unspecified";

                match opt_timeout_at(
                    deadline,
                    ReadyQueueManager::resolve_by_queue_name(
                        &self.name,
                        &self.queue_config,
                        egress_source,
                        egress_pool,
                        self.get_config_epoch(),
                    ),
                )
                .await
                {
                    Ok(site) => {
                        return site.insert(msg).await.map_err(|_| ReadyQueueFull.into());
                    }
                    Err(err) => {
                        log_disposition(LogDisposition {
                            kind: RecordType::TransientFailure,
                            msg: msg.clone(),
                            site: "",
                            peer_address: None,
                            response: Response {
                                code: 451,
                                enhanced_code: Some(EnhancedStatusCode {
                                    class: 4,
                                    subject: 4,
                                    detail: 4,
                                }),
                                content: format!("failed to resolve queue {}: {err:#}", self.name),
                                command: None,
                            },
                            egress_pool: None,
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: None,
                            tls_info: None,
                            source_address: None,
                            provider: self.queue_config.borrow().provider_name.as_deref(),
                            session_id: None,
                            recipient_list: None,
                        })
                        .await;
                        context.note(InsertReason::LoggedTransientFailure);
                        anyhow::bail!("failed to resolve queue {}: {err:#}", self.name);
                    }
                }
            }
            DeliveryProto::Null => {
                // We don't log anything here; this is in alignment with
                // our reception time behavior of not logging either.
                // We shouldn't get here unless someone re-bound a message
                // into the "null" queue, and there will be an AdminRebind
                // log entry recording that
                spawn("remove from spool", async move {
                    SpoolManager::remove_from_spool(*msg.id()).await
                })?;
                Ok(())
            }
            DeliveryProto::Maildir {
                maildir_path,
                dir_mode,
                file_mode,
            } => {
                let msg_data = opt_timeout_at(deadline, msg.data()).await?;

                let mut successes = vec![];
                let mut failures = vec![];

                let queue_name = msg.get_queue_name()?;
                let components = QueueNameComponents::parse(&queue_name);
                let sender = msg.sender()?;

                for recipient in msg.recipient_list()? {
                    let engine = TemplateEngine::new();
                    let expanded_maildir_path = engine.render(
                        "maildir_path",
                        maildir_path,
                        serde_json::json! ({
                            "meta": msg.get_meta_obj()?,
                            "queue": queue_name,
                            "campaign": components.campaign,
                            "tenant": components.tenant,
                            "domain": components.domain,
                            "routing_domain": components.routing_domain,
                            "local_part": recipient.user(),
                            "domain_part" : recipient.domain(),
                            "email": recipient.to_string(),
                            "sender_local_part": sender.user(),
                            "sender_domain_part": sender.domain(),
                            "sender_email": sender.to_string(),
                        }),
                    )?;

                    tracing::trace!(
                        "Deliver msg {} to maildir at {maildir_path} -> {expanded_maildir_path}",
                        msg.id(),
                    );
                    let dir_mode = *dir_mode;
                    let file_mode = *file_mode;

                    let name = self.name.to_string();
                    let result: anyhow::Result<String> = spawn_blocking_on(
                        "write to maildir",
                        {
                            let msg_data = msg_data.clone();
                            move || {
                                let mut md = maildir::Maildir::with_path(&expanded_maildir_path);
                                md.set_dir_mode(dir_mode);
                                md.set_file_mode(file_mode);
                                md.create_dirs().with_context(|| {
                                    format!(
                                        "failed to create maildir \
                                        {expanded_maildir_path} for queue {name}"
                                    )
                                })?;
                                Ok(md.store_new(&msg_data).with_context(|| {
                                    format!(
                                        "failed to store message to maildir \
                                        {expanded_maildir_path} for queue {name}"
                                    )
                                })?)
                            }
                        },
                        &get_main_runtime(),
                    )?
                    .await?;

                    match result {
                        Ok(id) => {
                            successes.push((recipient, id));
                        }
                        Err(err) => {
                            failures.push((recipient, err));
                        }
                    }
                }

                // Allow correlation of successful and failed attempts from this
                // same maildir writing "session"
                let session_id = Uuid::new_v4();

                if !successes.is_empty() {
                    let mut status = vec![];
                    for (recipient, id) in successes {
                        status.push(format!(
                            "{}: wrote to maildir with id={id}",
                            recipient.to_string()
                        ));
                    }
                    let status = status.join(", ");
                    log_disposition(LogDisposition {
                        kind: RecordType::Delivery,
                        msg: msg.clone(),
                        site: "",
                        peer_address: None,
                        response: Response {
                            code: 200,
                            enhanced_code: None,
                            content: status,
                            command: None,
                        },
                        egress_pool: None,
                        egress_source: None,
                        relay_disposition: None,
                        delivery_protocol: Some("Maildir"),
                        tls_info: None,
                        source_address: None,
                        provider: None,
                        session_id: Some(session_id),
                        recipient_list: None,
                    })
                    .await;
                }

                if !failures.is_empty() {
                    let mut remaining_recipient_list = vec![];
                    let mut status = vec![];
                    for (recipient, err) in failures {
                        status.push(format!(
                            "{}: failed to write to maildir: {err:#}",
                            recipient.to_string()
                        ));
                        remaining_recipient_list.push(recipient);
                    }
                    let status = status.join(", ");
                    log_disposition(LogDisposition {
                        kind: RecordType::TransientFailure,
                        msg: msg.clone(),
                        site: "",
                        peer_address: None,
                        response: Response {
                            code: 400,
                            enhanced_code: None,
                            content: status.clone(),
                            command: None,
                        },
                        egress_pool: None,
                        egress_source: None,
                        relay_disposition: None,
                        delivery_protocol: Some("Maildir"),
                        tls_info: None,
                        source_address: None,
                        provider: None,
                        session_id: Some(session_id),
                        recipient_list: None,
                    })
                    .await;
                    context.note(InsertReason::LoggedTransientFailure);
                    // Adjust for remaining recipients
                    msg.set_recipient_list(remaining_recipient_list)?;
                    anyhow::bail!("failed maildir store: {status}");
                } else {
                    // Every recipient in the batch was successful; this
                    // message has reached its final disposition and
                    // must now be removed
                    spawn("remove from spool", async move {
                        SpoolManager::remove_from_spool(*msg.id()).await
                    })?;
                    Ok(())
                }
            }
        }
    }

    /// Insert a newly received, or freshly loaded from spool, message
    /// into this queue
    #[instrument(fields(self.name), skip(self, msg))]
    pub async fn insert(
        self: &Arc<Self>,
        msg: Message,
        context: InsertContext,
        deadline: Option<Instant>,
    ) -> anyhow::Result<()> {
        *self.last_change.lock() = Instant::now();

        tracing::trace!("insert msg {}", msg.id());
        if let Some(b) = AdminBounceEntry::get_for_queue_name(&self.name) {
            let id = *msg.id();
            b.log(msg, Some(&self.name)).await;
            SpoolManager::remove_from_spool(id).await?;
            return Ok(());
        }

        if self.activity.is_shutting_down() {
            Self::save_if_needed_and_log(&msg, None).await;
            drop(msg);
            return Ok(());
        }

        match self.insert_delayed(msg.clone(), context.clone()).await? {
            InsertResult::Delayed => Ok(()),
            InsertResult::Ready(msg) => {
                self.insert_ready(msg.clone(), context, deadline).await?;
                Ok(())
            }
        }
    }

    /// Iterate over up to `take` messages in this queue.
    /// Not implemented for every queue strategy.
    pub fn iter(&self, take: Option<usize>) -> Vec<Message> {
        self.queue.iter(take)
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn get_config(&self) -> &ConfigHandle<QueueConfig> {
        &self.queue_config
    }

    pub fn get_last_change(&self) -> Instant {
        *self.last_change.lock()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

declare_event! {
pub static GET_Q_CONFIG_SIG: Multiple(
        "get_queue_config",
        domain: &'static str,
        tenant: Option<&'static str>,
        campaign: Option<&'static str>,
        routing_domain: Option<&'static str>,
    ) -> QueueConfig;
}

static QMAINT_COUNT: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "scheduled_queue_maintainer_count",
        "how many scheduled queues have active maintainer tasks"
    )
    .unwrap()
});

declare_event! {
pub static THROTTLE_INSERT_READY_SIG: Multiple(
    "throttle_insert_ready_queue",
    message: Message,
) -> ();
}
declare_event! {
static REBIND_MESSAGE_SIG: Single(
    "rebind_message",
    message: Message,
    rebind_request_data: HashMap<String, String>,
) -> ();
}
