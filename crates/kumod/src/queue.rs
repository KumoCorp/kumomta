use crate::egress_path::EgressPathManager;
use crate::egress_source::{EgressPool, EgressPoolRoundRobin};
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::lifecycle::{Activity, ShutdownSubcription};
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::spool::SpoolManager;
use anyhow::anyhow;
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
use timeq::{PopResult, TimeQ, TimerError};
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;

lazy_static::lazy_static! {
    static ref MANAGER: Mutex<QueueManager> = Mutex::new(QueueManager::new());
    static ref DELAY_GAUGE: IntGaugeVec = {
        prometheus::register_int_gauge_vec!("delayed_count", "number of messages in the delayed queue", &["queue"]).unwrap()
    };
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct QueueConfig {
    /// Base retry interval to use in exponential backoff
    #[serde(default = "QueueConfig::default_retry_interval")]
    retry_interval: usize,

    /// Optional cap on the computed retry interval.
    /// Set to the same number as retry_interval to
    /// prevent using exponential backoff
    #[serde(default)]
    max_retry_interval: Option<usize>,

    /// Limits how long a message can remain in the queue
    #[serde(default = "QueueConfig::default_max_age")]
    max_age: usize,

    /// Specifies which egress pool should be used when
    /// delivering these messages
    #[serde(default)]
    egress_pool: Option<String>,
}

impl LuaUserData for QueueConfig {}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            retry_interval: Self::default_retry_interval(),
            max_retry_interval: None,
            max_age: Self::default_max_age(),
            egress_pool: None,
        }
    }
}

impl QueueConfig {
    fn default_retry_interval() -> usize {
        60 * 20 // 20 minutes
    }

    fn default_max_age() -> usize {
        86400 * 7 // 1 week
    }

    pub fn get_max_age(&self) -> chrono::Duration {
        chrono::Duration::seconds(self.max_age as i64)
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
        let delay = self.retry_interval * 2usize.saturating_pow(attempt as u32);

        let delay = match self.max_retry_interval {
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
            if age >= config.max_age as i64 {
                return schedule;
            }
            schedule.push(delay);
        }
        unreachable!()
    }

    #[test]
    fn calc_due() {
        let config = QueueConfig {
            retry_interval: 2,
            max_retry_interval: None,
            max_age: 1024,
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
            retry_interval: 2,
            max_retry_interval: Some(8),
            max_age: 128,
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
            retry_interval: 2,
            max_retry_interval: None,
            max_age: 256,
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
            retry_interval: 1200,
            max_retry_interval: None,
            max_age: 3 * 3600,
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
    last_change: Instant,
    queue_config: QueueConfig,
    delayed_gauge: IntGauge,
    activity: Activity,
    rr: EgressPoolRoundRobin,
}

impl Drop for Queue {
    fn drop(&mut self) {
        if let Some(handle) = self.maintainer.take() {
            handle.abort();
        }
    }
}

impl Queue {
    pub async fn new(name: String) -> anyhow::Result<QueueHandle> {
        let mut config = load_config().await?;

        let components = QueueNameComponents::parse(&name);
        let queue_config: QueueConfig = config.call_callback(
            "get_queue_config",
            (components.domain, components.tenant, components.campaign),
        )?;

        let pool = EgressPool::resolve(queue_config.egress_pool.as_deref())?;
        let rr = EgressPoolRoundRobin::new(&pool);

        let delayed_gauge = DELAY_GAUGE.get_metric_with_label_values(&[&name])?;

        let activity = Activity::get()?;

        let handle = QueueHandle(Arc::new(Mutex::new(Queue {
            name: name.clone(),
            queue: TimeQ::new(),
            maintainer: None,
            last_change: Instant::now(),
            queue_config,
            delayed_gauge,
            activity,
            rr,
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
        Ok(handle)
    }

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
            if let Some(site) = EgressPathManager::get_opt(&self.name, source).await {
                site.lock().await.bounce_all(bounce).await;
            }
        }
    }

    pub async fn requeue_message(
        &mut self,
        msg: Message,
        increment_attempts: bool,
        delay: Option<chrono::Duration>,
    ) -> anyhow::Result<()> {
        let id = *msg.id();
        if increment_attempts {
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
                })
                .await;
                SpoolManager::remove_from_spool(id).await?;
                return Ok(());
            }
            msg.delay_by(delay).await?;
        } else if let Some(delay) = delay {
            msg.delay_by(delay).await?;
        } else {
            msg.delay_with_jitter(60).await?;
        }

        self.insert(msg).await?;

        Ok(())
    }

    async fn insert_delayed(&mut self, msg: Message) -> anyhow::Result<InsertResult> {
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

    async fn force_into_delayed(&mut self, msg: Message) -> anyhow::Result<()> {
        loop {
            msg.delay_with_jitter(60).await?;
            match self.insert_delayed(msg.clone()).await? {
                InsertResult::Delayed => return Ok(()),
                // Maybe delay_with_jitter computed an immediate
                // time? Let's try again
                InsertResult::Ready(_) => continue,
            }
        }
    }

    pub async fn save_if_needed(msg: &Message) -> anyhow::Result<()> {
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

    async fn insert_ready(&mut self, msg: Message) -> anyhow::Result<()> {
        let egress_source = self
            .rr
            .next()
            .ok_or_else(|| anyhow!("no sources in pool"))?;
        let site =
            EgressPathManager::resolve_by_queue_name(&self.name, &egress_source, &self.rr.name)
                .await?;
        let mut site = site.lock().await;
        site.insert(msg)
            .map_err(|_| anyhow!("no room in ready queue"))
    }

    pub async fn insert(&mut self, msg: Message) -> anyhow::Result<()> {
        self.last_change = Instant::now();

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
                if let Err(_err) = self.insert_ready(msg.clone()).await {
                    self.force_into_delayed(msg).await?;
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
    pub async fn insert(name: &str, msg: Message) -> anyhow::Result<()> {
        let entry = Self::resolve(name).await?;
        let mut entry = entry.lock().await;
        entry.insert(msg).await
    }

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

async fn maintain_named_queue(queue: &QueueHandle) -> anyhow::Result<()> {
    let mut sleep_duration = Duration::from_secs(60);
    let mut shutdown = ShutdownSubcription::get();

    loop {
        tokio::select! {
            _ = tokio::time::sleep(sleep_duration) => {}
            _ = shutdown.shutting_down() => {
            }
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
                sleep_duration = Duration::from_secs(5);
                for msg in q.queue.drain() {
                    Queue::save_if_needed_and_log(&msg).await;
                    drop(msg);
                }

                let path_mgr = EgressPathManager::get().await;
                if path_mgr.number_of_sites() == 0 {
                    tracing::debug!(
                        "{}: there are no more sites and the delayed queue is empty, reaping",
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

                    for msg in messages {
                        let egress_source =
                            q.rr.next().ok_or_else(|| anyhow!("no sources in pool"))?;

                        match EgressPathManager::resolve_by_queue_name(
                            &q.name,
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

                                match site.insert(msg.clone()) {
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
