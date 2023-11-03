use crate::egress_source::{EgressPool, EgressPoolRoundRobin, RoundRobinResult};
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::http_server::admin_suspend_v1::AdminSuspendEntry;
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::lua_deliver::LuaDeliveryProtocol;
use crate::ready_queue::ReadyQueueManager;
use crate::smtp_dispatcher::SmtpProtocol;
use crate::spool::SpoolManager;
use anyhow::Context;
use chrono::Utc;
use config::{load_config, CallbackSignature, LuaConfig};
use kumo_server_common::config_handle::ConfigHandle;
use kumo_server_lifecycle::{Activity, ShutdownSubcription};
use kumo_server_runtime::{rt_spawn, spawn, spawn_blocking};
use message::message::QueueNameComponents;
use message::Message;
use mlua::prelude::*;
use prometheus::{IntGauge, IntGaugeVec};
use rfc5321::{EnhancedStatusCode, Response};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use timeq::{PopResult, TimeQ, TimerError};
use tokio::sync::{Mutex, MutexGuard};
use tracing::instrument;

lazy_static::lazy_static! {
    static ref MANAGER: Mutex<QueueManager> = Mutex::new(QueueManager::new());
    static ref DELAY_GAUGE: IntGaugeVec = {
        prometheus::register_int_gauge_vec!("scheduled_count", "number of messages in the scheduled queue", &["queue"]).unwrap()
    };
    pub static ref GET_Q_CONFIG_SIG: CallbackSignature::<'static,
        (&'static str, Option<&'static str>, Option<&'static str>, Option<&'static str>),
        QueueConfig> = CallbackSignature::new_with_multiple("get_queue_config");
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum DeliveryProto {
    Smtp { smtp: SmtpProtocol },
    Maildir { maildir_path: std::path::PathBuf },
    Lua { custom_lua: LuaDeliveryProtocol },
}

impl DeliveryProto {
    pub fn metrics_protocol_name(&self) -> &'static str {
        match self {
            Self::Smtp { .. } => "smtp_client",
            Self::Maildir { .. } => "maildir",
            Self::Lua { .. } => "lua",
        }
    }

    pub fn ready_queue_name(&self) -> String {
        let proto_name = self.metrics_protocol_name();
        match self {
            Self::Smtp { .. } => proto_name.to_string(),
            Self::Maildir { maildir_path } => format!("{proto_name}:{}", maildir_path.display()),
            Self::Lua { custom_lua } => format!("{proto_name}:{}", custom_lua.constructor),
        }
    }
}

impl Default for DeliveryProto {
    fn default() -> Self {
        Self::Smtp {
            smtp: SmtpProtocol::default(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct QueueConfig {
    /// Base retry interval to use in exponential backoff
    #[serde(
        default = "QueueConfig::default_retry_interval",
        with = "humantime_serde"
    )]
    pub retry_interval: Duration,

    /// Optional cap on the computed retry interval.
    /// Set to the same number as retry_interval to
    /// prevent using exponential backoff
    #[serde(default, with = "humantime_serde")]
    pub max_retry_interval: Option<Duration>,

    /// Limits how long a message can remain in the queue
    #[serde(default = "QueueConfig::default_max_age", with = "humantime_serde")]
    pub max_age: Duration,

    /// Specifies which egress pool should be used when
    /// delivering these messages
    #[serde(default)]
    pub egress_pool: Option<String>,

    #[serde(default)]
    pub protocol: DeliveryProto,
}

impl LuaUserData for QueueConfig {}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            retry_interval: Self::default_retry_interval(),
            max_retry_interval: None,
            max_age: Self::default_max_age(),
            egress_pool: None,
            protocol: DeliveryProto::default(),
        }
    }
}

impl QueueConfig {
    fn default_retry_interval() -> Duration {
        Duration::from_secs(60 * 20) // 20 minutes
    }

    fn default_max_age() -> Duration {
        Duration::from_secs(86400 * 7) // 1 week
    }

    pub fn get_max_age(&self) -> chrono::Duration {
        chrono::Duration::from_std(self.max_age).unwrap()
    }

    pub fn infer_num_attempts(&self, age: chrono::Duration) -> u16 {
        let mut elapsed = chrono::Duration::seconds(0);
        let mut num_attempts = 0;

        loop {
            let delay = self.delay_for_attempt(num_attempts);
            if elapsed + delay > age {
                return num_attempts;
            }
            elapsed = elapsed + delay;
            num_attempts += 1;
        }
    }

    pub fn delay_for_attempt(&self, attempt: u16) -> chrono::Duration {
        let delay = self.retry_interval.as_secs() * 2u64.saturating_pow(attempt as u32);

        let delay = match self.max_retry_interval.map(|d| d.as_secs()) {
            None => delay,
            Some(limit) => delay.min(limit),
        };

        chrono::Duration::seconds(delay as i64)
    }

    pub fn compute_delay_based_on_age(
        &self,
        num_attempts: u16,
        age: chrono::Duration,
    ) -> Option<chrono::Duration> {
        let max_age = self.get_max_age();
        if age >= max_age {
            return None;
        }

        let overall_delay: i64 = (1..num_attempts)
            .into_iter()
            .map(|i| self.delay_for_attempt(i).num_seconds())
            .sum();
        let overall_delay = chrono::Duration::seconds(overall_delay);

        if overall_delay >= max_age {
            None
        } else if overall_delay <= age {
            // Ready now
            Some(chrono::Duration::seconds(0))
        } else {
            Some(overall_delay - age)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Returns the list of delays up until the max_age would be reached
    fn compute_schedule(config: &QueueConfig) -> Vec<i64> {
        let mut schedule = vec![];
        let mut age = 0;
        for attempt in 0.. {
            let delay = config.delay_for_attempt(attempt).num_seconds();
            age += delay;
            if age >= config.max_age.as_secs() as i64 {
                return schedule;
            }
            schedule.push(delay);
        }
        unreachable!()
    }

    #[test]
    fn calc_due() {
        let config = QueueConfig {
            retry_interval: Duration::from_secs(2),
            max_retry_interval: None,
            max_age: Duration::from_secs(1024),
            ..Default::default()
        };

        assert_eq!(
            compute_schedule(&config),
            vec![2, 4, 8, 16, 32, 64, 128, 256, 512]
        );
    }

    #[test]
    fn calc_due_capped() {
        let config = QueueConfig {
            retry_interval: Duration::from_secs(2),
            max_retry_interval: Some(Duration::from_secs(8)),
            max_age: Duration::from_secs(128),
            ..Default::default()
        };

        assert_eq!(
            compute_schedule(&config),
            vec![2, 4, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8]
        );
    }

    #[test]
    fn spool_in_delay() {
        let config = QueueConfig {
            retry_interval: Duration::from_secs(2),
            max_retry_interval: None,
            max_age: Duration::from_secs(256),
            ..Default::default()
        };

        let mut schedule = vec![];
        let mut age = 2;
        loop {
            let age_chrono = chrono::Duration::seconds(age);
            let num_attempts = config.infer_num_attempts(age_chrono);
            match config.compute_delay_based_on_age(num_attempts, age_chrono) {
                Some(delay) => schedule.push((age, num_attempts, delay.num_seconds())),
                None => break,
            }
            age += 4;
        }

        assert_eq!(
            schedule,
            vec![
                (2, 1, 0),
                (6, 2, 0),
                (10, 2, 0),
                (14, 3, 0),
                (18, 3, 0),
                (22, 3, 0),
                (26, 3, 0),
                (30, 4, 0),
                (34, 4, 0),
                (38, 4, 0),
                (42, 4, 0),
                (46, 4, 0),
                (50, 4, 0),
                (54, 4, 0),
                (58, 4, 0),
                (62, 5, 0),
                (66, 5, 0),
                (70, 5, 0),
                (74, 5, 0),
                (78, 5, 0),
                (82, 5, 0),
                (86, 5, 0),
                (90, 5, 0),
                (94, 5, 0),
                (98, 5, 0),
                (102, 5, 0),
                (106, 5, 0),
                (110, 5, 0),
                (114, 5, 0),
                (118, 5, 0),
                (122, 5, 0),
                (126, 6, 0),
                (130, 6, 0),
                (134, 6, 0),
                (138, 6, 0),
                (142, 6, 0),
                (146, 6, 0),
                (150, 6, 0),
                (154, 6, 0),
                (158, 6, 0),
                (162, 6, 0),
                (166, 6, 0),
                (170, 6, 0),
                (174, 6, 0),
                (178, 6, 0),
                (182, 6, 0),
                (186, 6, 0),
                (190, 6, 0),
                (194, 6, 0),
                (198, 6, 0),
                (202, 6, 0),
                (206, 6, 0),
                (210, 6, 0),
                (214, 6, 0),
                (218, 6, 0),
                (222, 6, 0),
                (226, 6, 0),
                (230, 6, 0),
                (234, 6, 0),
                (238, 6, 0),
                (242, 6, 0),
                (246, 6, 0),
                (250, 6, 0),
                (254, 7, 0)
            ]
        );
    }

    #[test]
    fn bigger_delay() {
        let config = QueueConfig {
            retry_interval: Duration::from_secs(1200),
            max_retry_interval: None,
            max_age: Duration::from_secs(3 * 3600),
            ..Default::default()
        };

        let mut schedule = vec![];
        let mut age = 1200;
        loop {
            let age_chrono = chrono::Duration::seconds(age);
            let num_attempts = config.infer_num_attempts(age_chrono);
            match config.compute_delay_based_on_age(num_attempts, age_chrono) {
                Some(delay) => schedule.push((age, num_attempts, delay.num_seconds())),
                None => break,
            }
            age += 1200;
        }

        assert_eq!(
            schedule,
            vec![
                (1200, 1, 0),
                (2400, 1, 0),
                (3600, 2, 0),
                (4800, 2, 0),
                (6000, 2, 0),
                (7200, 2, 0),
                (8400, 3, 0),
                (9600, 3, 0)
            ]
        );
    }
}

#[derive(Error, Debug)]
#[error("The Ready Queue is full")]
struct ReadyQueueFull;

#[derive(Error, Debug)]
#[error("The Ready Queue is suspend by configuration")]
pub struct ReadyQueueSuspended;

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
    last_change: Instant,
    queue_config: ConfigHandle<QueueConfig>,
    delayed_gauge: IntGauge,
    activity: Activity,
    rr: EgressPoolRoundRobin,
}

impl Queue {
    async fn call_get_queue_config(
        name: &str,
        config: &mut LuaConfig,
    ) -> anyhow::Result<QueueConfig> {
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
        let mut config = load_config().await?;
        let queue_config = Self::call_get_queue_config(&name, &mut config).await?;

        let pool = EgressPool::resolve(queue_config.egress_pool.as_deref(), &mut config).await?;
        let rr = EgressPoolRoundRobin::new(&pool);

        let delayed_gauge = DELAY_GAUGE.get_metric_with_label_values(&[&name])?;

        let activity = Activity::get(format!("Queue {name}"))?;

        let handle = QueueHandle(Arc::new(Mutex::new(Queue {
            name: name.clone(),
            queue: TimeQ::new(),
            last_change: Instant::now(),
            queue_config: ConfigHandle::new(queue_config),
            delayed_gauge,
            activity,
            rr,
        })));

        let queue_clone = handle.clone();
        rt_spawn(format!("maintain {name}"), move || {
            Ok(async move {
                if let Err(err) = maintain_named_queue(&queue_clone).await {
                    tracing::error!(
                        "maintain_named_queue {}: {err:#}",
                        queue_clone.lock().await.name
                    );
                }
            })
        })
        .await?;

        Ok(handle)
    }

    #[instrument(skip(self))]
    pub async fn bounce_all(&mut self, bounce: &AdminBounceEntry) {
        let msgs = self.queue.drain();
        let count = msgs.len();
        self.delayed_gauge.sub(count as i64);
        for msg in msgs {
            let msg = (*msg).clone();
            let id = *msg.id();
            bounce.log(msg, Some(&self.name)).await;
            SpoolManager::remove_from_spool(id).await.ok();
        }
    }

    async fn increment_attempts_and_update_delay(
        &self,
        msg: Message,
    ) -> anyhow::Result<Option<Message>> {
        let id = *msg.id();
        msg.increment_num_attempts();
        let delay = self
            .queue_config
            .borrow()
            .delay_for_attempt(msg.get_num_attempts());
        let jitter = (rand::random::<f32>() * 60.) - 30.0;
        let delay = chrono::Duration::seconds(delay.num_seconds() + jitter as i64);

        let now = Utc::now();
        let max_age = self.queue_config.borrow().get_max_age();
        let age = msg.age(now);
        let delayed_age = age + delay;
        if delayed_age > max_age {
            tracing::debug!("expiring {id} {delayed_age} > {max_age}");
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
                    content: format!("Next delivery time {delayed_age} > {max_age}"),
                    command: None,
                },
                egress_pool: self.queue_config.borrow().egress_pool.as_deref(),
                egress_source: None,
                relay_disposition: None,
                delivery_protocol: None,
                tls_info: None,
            })
            .await;
            SpoolManager::remove_from_spool(id).await?;
            return Ok(None);
        }
        tracing::trace!("increment_attempts_and_update_delay: delaying {id} by {delay}");
        msg.delay_by(delay).await?;
        Ok(Some(msg))
    }

    #[instrument(skip(self, msg))]
    pub async fn requeue_message(
        &mut self,
        msg: Message,
        increment_attempts: bool,
        delay: Option<chrono::Duration>,
    ) -> anyhow::Result<()> {
        if increment_attempts {
            match self.increment_attempts_and_update_delay(msg).await? {
                Some(msg) => {
                    return self.insert(msg).await;
                }
                None => return Ok(()),
            };
        } else if let Some(delay) = delay {
            msg.delay_by(delay).await?;
        } else {
            msg.delay_with_jitter(60).await?;
        }

        self.insert(msg).await?;

        Ok(())
    }

    #[instrument(skip(self, msg))]
    async fn insert_delayed(&mut self, msg: Message) -> anyhow::Result<InsertResult> {
        tracing::trace!("insert_delayed {}", msg.id());
        match self.queue.insert(Arc::new(msg.clone())) {
            Ok(_) => {
                self.delayed_gauge.inc();
                if let Err(err) = self.did_insert_delayed(msg.clone()).await {
                    tracing::error!("while shrinking: {}: {err:#}", msg.id());
                }
                Ok(InsertResult::Delayed)
            }
            Err(TimerError::Expired(msg)) => Ok(InsertResult::Ready((*msg).clone())),
            Err(err) => anyhow::bail!("queue insert error: {err:#?}"),
        }
    }

    #[instrument(skip(self, msg))]
    async fn force_into_delayed(&mut self, msg: Message) -> anyhow::Result<()> {
        tracing::trace!("force_into_delayed {}", msg.id());
        loop {
            match self.insert_delayed(msg.clone()).await? {
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
    pub async fn save_if_needed(msg: &Message) -> anyhow::Result<()> {
        tracing::trace!("save_if_needed {}", msg.id());
        if msg.needs_save() {
            msg.save().await?;
        }
        msg.shrink()?;
        Ok(())
    }

    pub async fn save_if_needed_and_log(msg: &Message) {
        if let Err(err) = Self::save_if_needed(msg).await {
            let id = msg.id();
            tracing::error!("error saving {id}: {err:#}");
        }
    }

    async fn did_insert_delayed(&self, msg: Message) -> anyhow::Result<()> {
        Self::save_if_needed(&msg).await
    }

    #[instrument(skip(self, msg))]
    async fn insert_ready(&mut self, msg: Message) -> anyhow::Result<()> {
        tracing::trace!("insert_ready {}", msg.id());

        let protocol = {
            let config = self.queue_config.borrow();
            config.protocol.clone()
        };

        match protocol {
            DeliveryProto::Smtp { .. } | DeliveryProto::Lua { .. } => {
                // rr_attempts is a bit gross; ideally rr.next would know how
                // to inspect the egress_path.suspended configuration and reflect
                // that in the RoundRobinResult, but it doesn't have a reference
                // to an nexisting configuration to inspect.  We could potentially
                // dynamically create an AdminSuspendReadyQEntry when resolving the
                // configuration in ReadyQueueManager::compute_config, but that
                // doesn't have a Duration that it can use for that record.
                // So for the moment, we're going to make a number of attempts
                // to figure out the source.
                let mut rr_attempts = self.rr.len();
                loop {
                    rr_attempts -= 1;

                    let egress_source = match self.rr.next(&self.name, &self.queue_config).await {
                        RoundRobinResult::Source(source) => source,
                        RoundRobinResult::Delay(duration) => {
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
                                    content: format!(
                                        "all possible sources for {} are suspended",
                                        self.name
                                    ),
                                    command: None,
                                },
                                egress_pool: None,
                                egress_source: None,
                                relay_disposition: None,
                                delivery_protocol: None,
                                tls_info: None,
                            })
                            .await;
                            msg.delay_by_and_jitter(duration).await?;
                            return self.force_into_delayed(msg).await;
                        }
                        RoundRobinResult::NoSources => {
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
                                    content: format!(
                                        "no non-zero-weighted sources available for {}. {:?}",
                                        self.name, self.rr,
                                    ),
                                    command: None,
                                },
                                egress_pool: None,
                                egress_source: None,
                                relay_disposition: None,
                                delivery_protocol: None,
                                tls_info: None,
                            })
                            .await;
                            anyhow::bail!(
                                "no non-zero-weighted sources available for {}",
                                self.name
                            );
                        }
                    };

                    match ReadyQueueManager::resolve_by_queue_name(
                        &self.name,
                        &self.queue_config,
                        &egress_source,
                        &self.rr.name,
                    )
                    .await
                    {
                        Ok(site) => {
                            let mut site = site.lock().await;
                            return site.insert(msg).await.map_err(|_| ReadyQueueFull.into());
                        }
                        Err(err) => {
                            if err.downcast_ref::<ReadyQueueSuspended>().is_some()
                                && rr_attempts > 0
                            {
                                continue;
                            }

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
                                    content: format!(
                                        "failed to resolve queue {}: {err:#}",
                                        self.name
                                    ),
                                    command: None,
                                },
                                egress_pool: None,
                                egress_source: None,
                                relay_disposition: None,
                                delivery_protocol: None,
                                tls_info: None,
                            })
                            .await;
                            anyhow::bail!("failed to resolve queue {}: {err:#}", self.name);
                        }
                    }
                }
            }
            DeliveryProto::Maildir { maildir_path } => {
                tracing::trace!(
                    "Deliver msg {} to maildir at {}",
                    msg.id(),
                    maildir_path.display()
                );
                let maildir_path = maildir_path.to_path_buf();

                msg.load_data_if_needed().await?;
                let name = self.name.to_string();
                let result: anyhow::Result<String> = spawn_blocking("write to maildir", {
                    let msg = msg.clone();
                    move || {
                        let md = maildir::Maildir::from(maildir_path.clone());
                        md.create_dirs().with_context(|| {
                            format!(
                                "creating dirs for maildir {maildir_path:?} in queue {}",
                                name
                            )
                        })?;
                        Ok(md.store_new(&msg.get_data())?)
                    }
                })?
                .await?;

                match result {
                    Ok(id) => {
                        log_disposition(LogDisposition {
                            kind: RecordType::Delivery,
                            msg: msg.clone(),
                            site: "",
                            peer_address: None,
                            response: Response {
                                code: 200,
                                enhanced_code: None,
                                content: format!("wrote to maildir with id={id}"),
                                command: None,
                            },
                            egress_pool: None,
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: Some("Maildir"),
                            tls_info: None,
                        })
                        .await;
                        spawn("remove from spool", async move {
                            SpoolManager::remove_from_spool(*msg.id()).await
                        })?;
                        Ok(())
                    }
                    Err(err) => {
                        log_disposition(LogDisposition {
                            kind: RecordType::TransientFailure,
                            msg: msg.clone(),
                            site: "",
                            peer_address: None,
                            response: Response {
                                code: 400,
                                enhanced_code: None,
                                content: format!("failed to write to maildir: {err:#}"),
                                command: None,
                            },
                            egress_pool: None,
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: Some("Maildir"),
                            tls_info: None,
                        })
                        .await;
                        anyhow::bail!("failed maildir store: {err:#}");
                    }
                }
            }
        }
    }

    #[instrument(fields(self.name), skip(self, msg))]
    pub async fn insert(&mut self, msg: Message) -> anyhow::Result<()> {
        loop {
            self.last_change = Instant::now();

            tracing::trace!("insert msg {}", msg.id());
            if let Some(b) = AdminBounceEntry::get_for_queue_name(&self.name) {
                let id = *msg.id();
                b.log(msg, Some(&self.name)).await;
                SpoolManager::remove_from_spool(id).await?;
                return Ok(());
            }

            if self.activity.is_shutting_down() {
                Self::save_if_needed_and_log(&msg).await;
                drop(msg);
                return Ok(());
            }

            match self.insert_delayed(msg.clone()).await? {
                InsertResult::Delayed => return Ok(()),
                InsertResult::Ready(msg) => {
                    // Don't promote to ready queue while suspended
                    if let Some(suspend) = AdminSuspendEntry::get_for_queue_name(&self.name) {
                        let remaining = suspend.get_duration();
                        msg.delay_by_and_jitter(remaining).await?;
                        // Continue and attempt to insert_delayed with
                        // the adjusted time
                        continue;
                    }

                    if let Err(err) = self.insert_ready(msg.clone()).await {
                        tracing::debug!("insert_ready: {err:#}");

                        if err.downcast_ref::<ReadyQueueFull>().is_none() {
                            // It was a legit error while trying to do something useful
                            match self.increment_attempts_and_update_delay(msg).await? {
                                Some(msg) => {
                                    self.force_into_delayed(msg).await?;
                                }
                                None => {}
                            }
                        } else {
                            // Queue is full; try again shortly
                            self.force_into_delayed(msg).await?;
                        }
                    }
                    return Ok(());
                }
            }
        }
    }

    pub fn get_config(&self) -> &ConfigHandle<QueueConfig> {
        &self.queue_config
    }
}

#[must_use]
enum InsertResult {
    Delayed,
    Ready(Message),
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
    #[instrument(skip(msg))]
    pub async fn insert(name: &str, msg: Message) -> anyhow::Result<()> {
        tracing::trace!("QueueManager::insert");
        let entry = Self::resolve(name).await?;
        let mut entry = entry.lock().await;
        entry.insert(msg).await
    }

    #[instrument]
    pub async fn resolve(name: &str) -> anyhow::Result<QueueHandle> {
        let mut mgr = MANAGER.lock().await;
        match mgr.named.get(name) {
            Some(e) => Ok((*e).clone()),
            None => {
                let entry = Queue::new(name.to_string()).await?;
                mgr.named.insert(name.to_string(), entry.clone());
                Ok(entry)
            }
        }
    }

    pub async fn get_opt(name: &str) -> Option<QueueHandle> {
        let mgr = MANAGER.lock().await;
        mgr.named.get(name).cloned()
    }

    pub async fn all_queue_names() -> Vec<String> {
        let mgr = Self::get().await;
        mgr.named.keys().map(|s| s.to_string()).collect()
    }

    async fn get() -> MutexGuard<'static, Self> {
        MANAGER.lock().await
    }
}

#[instrument(skip(queue))]
async fn maintain_named_queue(queue: &QueueHandle) -> anyhow::Result<()> {
    let mut sleep_duration = Duration::from_secs(60);
    let mut shutdown = ShutdownSubcription::get();
    let mut memory = kumo_server_memory::subscribe_to_memory_status_changes();
    let mut last_config_refresh = Instant::now();

    loop {
        tokio::select! {
            _ = tokio::time::sleep(sleep_duration) => {}
            _ = shutdown.shutting_down() => {}
            _ = memory.changed() => {}
        };

        {
            let mut q = queue.lock().await;
            tracing::debug!(
                "maintaining queue {} which has {} entries",
                q.name,
                q.queue.len()
            );

            if let Some(b) = AdminBounceEntry::get_for_queue_name(&q.name) {
                q.bounce_all(&b).await;
            }

            if q.activity.is_shutting_down() {
                sleep_duration = Duration::from_secs(1);
                for msg in q.queue.drain() {
                    Queue::save_if_needed_and_log(&msg).await;
                    drop(msg);
                }

                let queue_mgr = ReadyQueueManager::get().await;
                if queue_mgr.number_of_queues() == 0 {
                    tracing::debug!(
                        "{}: there are no more queues and the delayed queue is empty, reaping",
                        q.name
                    );
                    let mut mgr = QueueManager::get().await;
                    mgr.named.remove(&q.name);
                    return Ok(());
                }
                continue;
            }

            if last_config_refresh.elapsed() >= Duration::from_secs(60) {
                last_config_refresh = Instant::now();
                if let Ok(mut config) = load_config().await {
                    if let Ok(queue_config) =
                        Queue::call_get_queue_config(&q.name, &mut config).await
                    {
                        q.queue_config.update(queue_config);
                    }
                }
            }

            match q.queue.pop() {
                PopResult::Items(messages) => {
                    q.delayed_gauge.sub(messages.len() as i64);
                    tracing::trace!("{} msgs are now ready", messages.len());

                    for msg in messages {
                        let msg = (*msg).clone();

                        if let Err(err) = q.insert_ready(msg.clone()).await {
                            tracing::debug!("insert_ready: {err:#}");

                            if err.downcast_ref::<ReadyQueueFull>().is_none() {
                                // It was a legit error while trying to do something useful
                                match q.increment_attempts_and_update_delay(msg).await? {
                                    Some(msg) => {
                                        q.force_into_delayed(msg).await?;
                                    }
                                    None => {}
                                }
                            } else {
                                // Queue is full; try again shortly
                                q.force_into_delayed(msg).await?;
                            }
                        }
                    }
                }
                PopResult::Sleep(duration) => {
                    // We sleep at most 1 minute in case some other actor
                    // re-inserts a message with ~1 minute delay. If we were
                    // sleeping for 4 hours, we wouldn't wake up soon enough
                    // to notice and dispatch it.
                    sleep_duration = duration.min(Duration::from_secs(60));
                }
                PopResult::Empty => {
                    sleep_duration = Duration::from_secs(60);

                    let mut mgr = QueueManager::get().await;
                    if q.last_change.elapsed() > Duration::from_secs(60 * 10) {
                        mgr.named.remove(&q.name);
                        drop(mgr);
                        tracing::debug!("idling out queue {}", q.name);
                        // Remove any metrics that go with it, so that we don't
                        // end up using a lot of memory remembering stats from
                        // what might be a long tail of tiny domains forever.
                        DELAY_GAUGE.remove_label_values(&[&q.name]).ok();
                        return Ok(());
                    }
                }
            }
        }
    }
}
