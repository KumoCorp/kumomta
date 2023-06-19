use crate::egress_source::{EgressPool, EgressPoolRoundRobin};
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::lifecycle::{Activity, ShutdownSubcription};
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::lua_deliver::LuaDeliveryProtocol;
use crate::ready_queue::ReadyQueueManager;
use crate::runtime::{rt_spawn, spawn, spawn_blocking};
use crate::spool::SpoolManager;
use anyhow::{anyhow, Context};
use chrono::Utc;
use config::load_config;
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
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum DeliveryProto {
    Smtp,
    Maildir { maildir_path: std::path::PathBuf },
    Lua { custom_lua: LuaDeliveryProtocol },
}

impl Default for DeliveryProto {
    fn default() -> Self {
        Self::Smtp
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
    queue_config: QueueConfig,
    delayed_gauge: IntGauge,
    activity: Activity,
    rr: EgressPoolRoundRobin,
}

impl Queue {
    pub async fn new(name: String) -> anyhow::Result<QueueHandle> {
        let mut config = load_config().await?;

        let components = QueueNameComponents::parse(&name);
        let queue_config: QueueConfig = config
            .async_call_callback(
                "get_queue_config",
                (components.domain, components.tenant, components.campaign),
            )
            .await?;

        let pool = EgressPool::resolve(queue_config.egress_pool.as_deref(), &mut config).await?;
        let rr = EgressPoolRoundRobin::new(&pool);

        let delayed_gauge = DELAY_GAUGE.get_metric_with_label_values(&[&name])?;

        let activity = Activity::get()?;

        let handle = QueueHandle(Arc::new(Mutex::new(Queue {
            name: name.clone(),
            queue: TimeQ::new(),
            last_change: Instant::now(),
            queue_config,
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

        let sources = self.rr.all_sources();
        for source in &sources {
            if let Some(site) =
                ReadyQueueManager::get_opt(&self.name, &self.queue_config, source).await
            {
                site.lock().await.bounce_all(bounce).await;
            }
        }
    }

    async fn increment_attempts_and_update_delay(
        &self,
        msg: Message,
    ) -> anyhow::Result<Option<Message>> {
        let id = *msg.id();
        msg.increment_num_attempts();
        let delay = self.queue_config.delay_for_attempt(msg.get_num_attempts());
        let jitter = (rand::random::<f32>() * 60.) - 30.0;
        let delay = chrono::Duration::seconds(delay.num_seconds() + jitter as i64);

        let now = Utc::now();
        let max_age = self.queue_config.get_max_age();
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
                egress_pool: self.queue_config.egress_pool.as_deref(),
                egress_source: None,
                relay_disposition: None,
                delivery_protocol: None,
            })
            .await;
            SpoolManager::remove_from_spool(id).await?;
            return Ok(None);
        }
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
        match &self.queue_config.protocol {
            DeliveryProto::Smtp | DeliveryProto::Lua { .. } => {
                let egress_source = self
                    .rr
                    .next()
                    .ok_or_else(|| anyhow!("no sources in pool"))?;
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
                        site.insert(msg).await.map_err(|_| ReadyQueueFull.into())
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
                                content: format!("failed to resolve {}: {err:#}", self.name),
                                command: None,
                            },
                            egress_pool: None,
                            egress_source: None,
                            relay_disposition: None,
                            delivery_protocol: None,
                        })
                        .await;
                        anyhow::bail!("failed to resolve {}: {err:#}", self.name);
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
            InsertResult::Delayed => Ok(()),
            InsertResult::Ready(msg) => {
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
                Ok(())
            }
        }
    }

    pub fn get_config(&self) -> &QueueConfig {
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
    let mut memory = crate::memory::subscribe_to_memory_status_changes();

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

            let now = Utc::now();
            match q.queue.pop() {
                PopResult::Items(messages) => {
                    q.delayed_gauge.sub(messages.len() as i64);
                    let max_age = q.queue_config.get_max_age();
                    tracing::trace!("{} msgs are now ready", messages.len());

                    for msg in messages {
                        let egress_source =
                            q.rr.next().ok_or_else(|| anyhow!("no sources in pool"))?;

                        match ReadyQueueManager::resolve_by_queue_name(
                            &q.name,
                            &q.queue_config,
                            &egress_source,
                            &q.rr.name,
                        )
                        .await
                        {
                            Ok(site) => {
                                let mut site = site.lock().await;

                                let msg = (*msg).clone();
                                let id = *msg.id();

                                let age = msg.age(now);
                                if age >= max_age {
                                    // TODO: log failure due to expiration
                                    tracing::debug!("expiring {id} {age} > {max_age}");
                                    SpoolManager::remove_from_spool(id).await?;
                                    continue;
                                }

                                match site.insert(msg.clone()).await {
                                    Ok(_) => {}
                                    Err(_) => loop {
                                        msg.delay_with_jitter(60).await?;
                                        if matches!(
                                            q.insert_delayed(msg.clone()).await?,
                                            InsertResult::Delayed
                                        ) {
                                            break;
                                        }
                                    },
                                }
                            }
                            Err(err) => {
                                tracing::error!("Failed to resolve {}: {err:#}", q.name);
                                log_disposition(LogDisposition {
                                    kind: RecordType::TransientFailure,
                                    msg: (*msg).clone(),
                                    site: "",
                                    peer_address: None,
                                    response: Response {
                                        code: 451,
                                        enhanced_code: Some(EnhancedStatusCode {
                                            class: 4,
                                            subject: 4,
                                            detail: 4,
                                        }),
                                        content: format!("failed to resolve {}: {err:#}", q.name),
                                        command: None,
                                    },
                                    egress_pool: None,
                                    egress_source: None,
                                    relay_disposition: None,
                                    delivery_protocol: None,
                                })
                                .await;
                                q.force_into_delayed((*msg).clone()).await?;
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
