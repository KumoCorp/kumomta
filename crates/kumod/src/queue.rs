use crate::egress_source::{EgressPool, EgressPoolRoundRobin, RoundRobinResult};
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::http_server::admin_rebind_v1::AdminRebindEntry;
use crate::http_server::admin_suspend_v1::AdminSuspendEntry;
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::lua_deliver::LuaDeliveryProtocol;
use crate::ready_queue::{ReadyQueueManager, ReadyQueueName};
use crate::smtp_dispatcher::SmtpProtocol;
use crate::spool::SpoolManager;
use anyhow::Context;
use chrono::{DateTime, Utc};
use config::{load_config, CallbackSignature, LuaConfig};
use crossbeam_skiplist::SkipSet;
use kumo_prometheus::{
    PruningIntCounter, PruningIntCounterVec, PruningIntGauge, PruningIntGaugeVec,
};
use kumo_server_common::config_handle::ConfigHandle;
use kumo_server_lifecycle::{Activity, ShutdownSubcription};
use kumo_server_runtime::{get_main_runtime, spawn, spawn_blocking_on, Runtime};
use message::message::{QueueNameComponents, WeakMessage};
use message::Message;
use mlua::prelude::*;
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::FairMutex as StdMutex;
use prometheus::{Histogram, IntCounter, IntGauge};
use rfc5321::{EnhancedStatusCode, Response};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};
use thiserror::Error;
use throttle::{ThrottleResult, ThrottleSpec};
use timeq::{PopResult, TimeQ, TimerError};
use tokio::sync::Notify;
use tracing::instrument;

lazy_static::lazy_static! {
    static ref MANAGER: StdMutex<QueueManager> = StdMutex::new(QueueManager::new());
    static ref SCHEDULED_QUEUE_COUNT: IntGauge = {
        prometheus::register_int_gauge!("scheduled_queue_count",
            "how many scheduled queues are tracked by the QueueManager").unwrap()
    };
    static ref QMAINT_COUNT: IntGauge = {
        prometheus::register_int_gauge!("scheduled_queue_maintainer_count",
            "how many scheduled queues have active maintainer tasks").unwrap()
    };
    static ref TOTAL_QMAINT_RUNS: IntCounter = {
        prometheus::register_int_counter!(
            "total_qmaint_runs",
            "total number of times a scheduled queue maintainer was run"
            ).unwrap()
    };

    pub static ref QMAINT_RUNTIME: Runtime = Runtime::new(
        "qmaint", |cpus| cpus/4, &QMAINT_THREADS).unwrap();

    pub static ref GET_Q_CONFIG_SIG: CallbackSignature::<'static,
        (&'static str, Option<&'static str>, Option<&'static str>, Option<&'static str>),
        QueueConfig> = CallbackSignature::new_with_multiple("get_queue_config");
    pub static ref THROTTLE_INSERT_READY_SIG: CallbackSignature::<'static,
        Message,
        ()> = CallbackSignature::new_with_multiple("throttle_insert_ready_queue");
    static ref REBIND_MESSAGE_SIG: CallbackSignature::<'static,
        (Message, HashMap<String, String>), ()> = CallbackSignature::new("rebind_message");

    static ref SINGLETON_WHEEL: Arc<StdMutex<TimeQ<WeakMessage>>> = Arc::new(StdMutex::new(TimeQ::new()));
}

static DELAY_GAUGE: Lazy<PruningIntGaugeVec> = Lazy::new(|| {
    PruningIntGaugeVec::register(
        "scheduled_count",
        "number of messages in the scheduled queue",
        &["queue"],
    )
});
static TENANT_GAUGE: Lazy<PruningIntGaugeVec> = Lazy::new(|| {
    PruningIntGaugeVec::register(
        "scheduled_by_tenant",
        "number of messages in the scheduled queue for a specific tenant",
        &["tenant"],
    )
});
static TENANT_CAMPAIGN_GAUGE: Lazy<PruningIntGaugeVec> = Lazy::new(|| {
    PruningIntGaugeVec::register(
        "scheduled_by_tenant_campaign",
        "number of messages in the scheduled queue for a specific tenant and campaign combination",
        &["tenant", "campaign"],
    )
});
static DOMAIN_GAUGE: Lazy<PruningIntGaugeVec> = Lazy::new(|| {
    PruningIntGaugeVec::register(
        "scheduled_by_domain",
        "number of messages in the scheduled queue for a specific domain",
        &["domain"],
    )
});
static DELAY_DUE_TO_READY_QUEUE_FULL_COUNTER: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "delayed_due_to_ready_queue_full",
        "number of times a message was delayed due to the corresponding ready queue being full",
        &["queue"],
    )
});
static DELAY_DUE_TO_MESSAGE_RATE_THROTTLE_COUNTER: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "delayed_due_to_message_rate_throttle",
        "number of times a message was delayed due to max_message_rate",
        &["queue"],
    )
});
static DELAY_DUE_TO_THROTTLE_INSERT_READY_COUNTER: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "delayed_due_to_throttle_insert_ready",
        "number of times a message was delayed due throttle_insert_ready_queue event",
        &["queue"],
    )
});
static RESOLVE_LATENCY: Lazy<Histogram> = Lazy::new(|| {
    prometheus::register_histogram!(
        "queue_resolve_latency",
        "latency of QueueManager::resolve operations",
    )
    .unwrap()
});
static INSERT_LATENCY: Lazy<Histogram> = Lazy::new(|| {
    prometheus::register_histogram!(
        "queue_insert_latency",
        "latency of QueueManager::insert operations",
    )
    .unwrap()
});

static STARTED_SINGLETON_WHEEL: Once = Once::new();
static QMAINT_THREADS: AtomicUsize = AtomicUsize::new(0);
const ZERO_DURATION: Duration = Duration::from_secs(0);
const ONE_SECOND: Duration = Duration::from_secs(1);
const ONE_DAY: Duration = Duration::from_secs(86400);
const ONE_MINUTE: Duration = Duration::from_secs(60);
const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

struct ScheduledMetrics {
    name: Arc<String>,
    scheduled: PruningIntGauge,
    by_domain: PruningIntGauge,
    by_tenant: Option<PruningIntGauge>,
    by_tenant_campaign: Option<PruningIntGauge>,
    delay_due_to_message_rate_throttle: OnceCell<PruningIntCounter>,
    delay_due_to_throttle_insert_ready: OnceCell<PruningIntCounter>,
    delay_due_to_ready_queue_full: OnceCell<PruningIntCounter>,
}

impl ScheduledMetrics {
    pub fn new(name: Arc<String>) -> Self {
        let components = QueueNameComponents::parse(&name);
        let scheduled = DELAY_GAUGE.with_label_values(&[&name]);

        let by_domain = DOMAIN_GAUGE.with_label_values(&[components.domain]);
        let by_tenant = components
            .tenant
            .map(|tenant| TENANT_GAUGE.with_label_values(&[tenant]));
        let by_tenant_campaign = match &components.campaign {
            Some(campaign) => Some(TENANT_CAMPAIGN_GAUGE.with_label_values(&[
                components.tenant.as_ref().map(|s| s.as_ref()).unwrap_or(""),
                campaign,
            ])),
            None => None,
        };

        Self {
            name,
            by_domain,
            by_tenant,
            by_tenant_campaign,
            scheduled,
            delay_due_to_message_rate_throttle: OnceCell::new(),
            delay_due_to_throttle_insert_ready: OnceCell::new(),
            delay_due_to_ready_queue_full: OnceCell::new(),
        }
    }

    pub fn delay_due_to_message_rate_throttle(&self) -> &PruningIntCounter {
        self.delay_due_to_message_rate_throttle.get_or_init(|| {
            DELAY_DUE_TO_MESSAGE_RATE_THROTTLE_COUNTER.with_label_values(&[&self.name])
        })
    }
    pub fn delay_due_to_throttle_insert_ready(&self) -> &PruningIntCounter {
        self.delay_due_to_throttle_insert_ready.get_or_init(|| {
            DELAY_DUE_TO_THROTTLE_INSERT_READY_COUNTER.with_label_values(&[&self.name])
        })
    }
    pub fn delay_due_to_ready_queue_full(&self) -> &PruningIntCounter {
        self.delay_due_to_ready_queue_full
            .get_or_init(|| DELAY_DUE_TO_READY_QUEUE_FULL_COUNTER.with_label_values(&[&self.name]))
    }

    pub fn inc(&self) {
        self.scheduled.inc();
        self.by_domain.inc();
        self.by_tenant.as_ref().map(|m| m.inc());
        self.by_tenant_campaign.as_ref().map(|m| m.inc());
    }

    pub fn sub(&self, amount: i64) {
        self.scheduled.sub(amount);
        self.by_domain.sub(amount);
        self.by_tenant.as_ref().map(|m| m.sub(amount));
        self.by_tenant_campaign.as_ref().map(|m| m.sub(amount));
    }
}

pub fn set_qmaint_threads(n: usize) {
    QMAINT_THREADS.store(n, Ordering::SeqCst);
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

#[derive(Deserialize, Serialize, Debug, Clone, FromLua, Default, Copy, PartialEq, Eq)]
pub enum QueueStrategy {
    #[default]
    TimerWheel,
    SkipList,
    SingletonTimerWheel,
}

#[derive(Deserialize, Serialize, Debug, Clone, FromLua)]
#[serde(deny_unknown_fields)]
pub struct QueueConfig {
    /// Base retry interval to use in exponential backoff
    #[serde(
        default = "QueueConfig::default_retry_interval",
        with = "duration_serde"
    )]
    pub retry_interval: Duration,

    /// Optional cap on the computed retry interval.
    /// Set to the same number as retry_interval to
    /// prevent using exponential backoff
    #[serde(default, with = "duration_serde")]
    pub max_retry_interval: Option<Duration>,

    /// Limits how long a message can remain in the queue
    #[serde(default = "QueueConfig::default_max_age", with = "duration_serde")]
    pub max_age: Duration,

    /// Specifies which egress pool should be used when
    /// delivering these messages
    #[serde(default)]
    pub egress_pool: Option<String>,

    /// The rate at which messages are allowed to move from
    /// the scheduled queue and into the ready queue
    #[serde(default)]
    pub max_message_rate: Option<ThrottleSpec>,

    #[serde(default)]
    pub protocol: DeliveryProto,

    /// How long to wait after the queue is idle before reaping
    /// and removing the scheduled queue from memory
    #[serde(
        default = "QueueConfig::default_reap_interval",
        with = "duration_serde"
    )]
    pub reap_interval: Duration,

    /// How long to wait between calls to get_queue_config for
    /// any given scheduled queue. Making this longer uses fewer
    /// resources (in aggregate) but means that it will take longer
    /// to detect and adjust to changes in the queue configuration.
    #[serde(
        default = "QueueConfig::default_refresh_interval",
        with = "duration_serde"
    )]
    pub refresh_interval: Duration,

    #[serde(with = "duration_serde", default)]
    pub timerwheel_tick_interval: Option<Duration>,

    #[serde(default)]
    pub strategy: QueueStrategy,
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
            max_message_rate: None,
            reap_interval: Self::default_reap_interval(),
            refresh_interval: Self::default_refresh_interval(),
            strategy: QueueStrategy::default(),
            timerwheel_tick_interval: None,
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

    fn default_reap_interval() -> Duration {
        TEN_MINUTES
    }

    fn default_refresh_interval() -> Duration {
        ONE_MINUTE
    }

    pub fn get_max_age(&self) -> chrono::Duration {
        chrono::Duration::from_std(self.max_age).unwrap()
    }

    pub fn infer_num_attempts(&self, age: chrono::Duration) -> u16 {
        let mut elapsed = chrono::Duration::zero();
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

        chrono::Duration::try_seconds(delay as i64).unwrap_or_else(chrono::Duration::zero)
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
        let overall_delay = chrono::Duration::try_seconds(overall_delay)?;

        if overall_delay >= max_age {
            None
        } else if overall_delay <= age {
            // Ready now
            Some(chrono::Duration::zero())
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
    fn calc_due_defaults() {
        let config = QueueConfig {
            retry_interval: Duration::from_secs(60 * 20),
            max_retry_interval: None,
            max_age: Duration::from_secs(86400),
            ..Default::default()
        };

        assert_eq!(
            compute_schedule(&config),
            vec![1200, 2400, 4800, 9600, 19200, 38400],
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
            let age_chrono = chrono::Duration::try_seconds(age).expect("age to be in range");
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
            let age_chrono = chrono::Duration::try_seconds(age).expect("age to be in range");
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

type QueueHandle = Arc<Queue>;

struct CachedReadyQueueName {
    name: ReadyQueueName,
    /// queue_config.generation()
    generation: usize,
}

#[derive(Debug)]
struct DelayedEntry(Message);

impl DelayedEntry {
    /// Get the due time with lower granularity than the underlying
    /// timestamp allows.
    /// Here it is 1 second.  For sites with very large
    /// scheduled queues and reasonable retry intervals
    /// it is desirable to reduce the granularity beacuse
    /// it makes the cost of the skiplist insertion
    /// cheaper when multiple items compare equal: we can insert
    /// when we find the start of a batch with the same second
    fn get_bucketed_due(&self) -> i64 {
        self.0.get_due().map(|d| d.timestamp()).unwrap_or(0)
    }

    fn compute_delay(&self, now: DateTime<Utc>) -> Duration {
        let due = self.get_bucketed_due();
        let now_ts = now.timestamp();
        Duration::from_secs(due.saturating_sub(now_ts).max(0) as u64)
    }
}

impl PartialEq for DelayedEntry {
    fn eq(&self, other: &DelayedEntry) -> bool {
        self.get_bucketed_due().eq(&other.get_bucketed_due())
    }
}
impl Eq for DelayedEntry {}
impl PartialOrd for DelayedEntry {
    fn partial_cmp(&self, other: &DelayedEntry) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DelayedEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get_bucketed_due().cmp(&other.get_bucketed_due())
    }
}

#[must_use]
enum QueueInsertResult {
    Inserted { should_notify: bool },
    Full(Message),
}

enum QueueStructure {
    TimerWheel(StdMutex<TimeQ<Message>>),
    SkipList(SkipSet<DelayedEntry>),
    SingletonTimerWheel(StdMutex<HashSet<Message>>),
}

impl QueueStructure {
    fn new(strategy: QueueStrategy) -> Self {
        match strategy {
            QueueStrategy::TimerWheel => Self::TimerWheel(StdMutex::new(TimeQ::new())),
            QueueStrategy::SkipList => Self::SkipList(SkipSet::new()),
            QueueStrategy::SingletonTimerWheel => {
                Self::SingletonTimerWheel(StdMutex::new(HashSet::new()))
            }
        }
    }

    fn pop(&self) -> (Vec<Message>, Option<Duration>) {
        match self {
            Self::TimerWheel(q) => match q.lock().pop() {
                PopResult::Items(messages) => (messages, None),
                PopResult::Sleep(_) | PopResult::Empty => (vec![], None),
            },
            Self::SkipList(q) => {
                let now = Utc::now();
                let mut messages = vec![];
                let mut sleep_duration = None;

                while let Some(entry) = q.front() {
                    let delay = entry.compute_delay(now);
                    if delay == ZERO_DURATION {
                        entry.remove();
                        messages.push(entry.0.clone());
                    } else {
                        sleep_duration = Some(delay);
                        break;
                    }
                }

                (messages, sleep_duration)
            }
            Self::SingletonTimerWheel(_) => (vec![], None),
        }
    }

    fn drain(&self) -> Vec<Message> {
        match self {
            Self::TimerWheel(q) => q.lock().drain(),
            Self::SkipList(q) => {
                let mut msgs = vec![];
                while let Some(entry) = q.pop_front() {
                    msgs.push((*entry).0.clone());
                }
                msgs
            }
            Self::SingletonTimerWheel(q) => q.lock().drain().collect(),
        }
    }

    fn insert(&self, msg: Message) -> QueueInsertResult {
        match self {
            Self::TimerWheel(q) => match q.lock().insert(msg) {
                Ok(()) => QueueInsertResult::Inserted {
                    // We never notify for TimerWheel because we always tick
                    // on a regular(ish) schedule
                    should_notify: false,
                },
                Err(TimerError::Expired(msg)) => QueueInsertResult::Full(msg),
                Err(TimerError::NotFound) => unreachable!(),
            },
            Self::SkipList(q) => {
                let due = q.front().map(|entry| entry.get_bucketed_due());
                q.insert(DelayedEntry(msg));
                let now_due = q.front().map(|entry| entry.get_bucketed_due());
                QueueInsertResult::Inserted {
                    // Only notify the maintainer if it now needs to wake up
                    // sooner than it previously thought. In particular,
                    // we do not want to wake up for every message insertion,
                    // as that would generally be a waste of effort and bog
                    // down the system without gain.
                    should_notify: now_due < due,
                }
            }
            Self::SingletonTimerWheel(q) => {
                match SINGLETON_WHEEL.lock().insert(msg.weak()) {
                    Ok(()) => {
                        q.lock().insert(msg);
                        STARTED_SINGLETON_WHEEL.call_once(|| {
                            QMAINT_RUNTIME
                                .spawn_non_blocking("singleton_wheel".to_string(), move || {
                                    Ok(async move {
                                        if let Err(err) = Queue::run_singleton_wheel().await {
                                            tracing::error!("run_singleton_wheel: {err:#}");
                                        }
                                    })
                                })
                                .expect("failed to spawn singleton_wheel");
                        });

                        QueueInsertResult::Inserted {
                            // We never notify for TimerWheel because we always tick
                            // on a regular(ish) schedule
                            should_notify: false,
                        }
                    }
                    Err(TimerError::Expired(_weak_msg)) => QueueInsertResult::Full(msg),
                    Err(TimerError::NotFound) => unreachable!(),
                }
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::TimerWheel(q) => q.lock().len(),
            Self::SkipList(q) => q.len(),
            Self::SingletonTimerWheel(q) => q.lock().len(),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::TimerWheel(q) => q.lock().is_empty(),
            Self::SkipList(q) => q.is_empty(),
            Self::SingletonTimerWheel(q) => q.lock().is_empty(),
        }
    }

    fn is_timer_wheel(&self) -> bool {
        matches!(self, Self::TimerWheel(_))
    }

    fn strategy(&self) -> QueueStrategy {
        match self {
            Self::TimerWheel(_) => QueueStrategy::TimerWheel,
            Self::SkipList(_) => QueueStrategy::SkipList,
            Self::SingletonTimerWheel(_) => QueueStrategy::SingletonTimerWheel,
        }
    }
}

pub struct Queue {
    name: Arc<String>,
    queue: QueueStructure,
    notify_maintainer: Arc<Notify>,
    last_change: StdMutex<Instant>,
    queue_config: ConfigHandle<QueueConfig>,
    metrics: OnceCell<ScheduledMetrics>,
    activity: Activity,
    rr: EgressPoolRoundRobin,
    ready_queue_names: StdMutex<HashMap<String, Arc<CachedReadyQueueName>>>,
    next_config_refresh: StdMutex<Instant>,
    warned_strategy_change: AtomicBool,
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

        let activity = Activity::get(format!("Queue {name}"))?;
        let strategy = queue_config.strategy;
        let next_config_refresh = StdMutex::new(Instant::now() + queue_config.refresh_interval);
        let name = Arc::new(name);

        let handle = Arc::new(Queue {
            name: name.clone(),
            queue: QueueStructure::new(strategy),
            last_change: StdMutex::new(Instant::now()),
            queue_config: ConfigHandle::new(queue_config),
            notify_maintainer: Arc::new(Notify::new()),
            metrics: OnceCell::new(),
            activity,
            rr,
            ready_queue_names: StdMutex::new(HashMap::new()),
            next_config_refresh,
            warned_strategy_change: AtomicBool::new(false),
        });

        if !matches!(strategy, QueueStrategy::SingletonTimerWheel) {
            let queue_clone = handle.clone();
            QMAINT_RUNTIME
                .spawn_non_blocking(format!("maintain {name}"), move || {
                    Ok(async move {
                        QMAINT_COUNT.inc();
                        if let Err(err) = maintain_named_queue(&queue_clone).await {
                            tracing::error!("maintain_named_queue {}: {err:#}", queue_clone.name);
                        }
                        QMAINT_COUNT.dec();
                    })
                })
                .expect("failed to spawn maintainer");
        }

        Ok(handle)
    }

    async fn queue_config_maintainer() {
        let mut shutdown = ShutdownSubcription::get();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    Self::check_config_refresh().await;
                }
                _ = shutdown.shutting_down() => {
                    tracing::info!("queue_config_maintainer stopping");
                    return;
                }
            }
        }
    }

    async fn check_config_refresh() {
        let now = Instant::now();

        tracing::debug!("check_config_refresh begins");
        let names = QueueManager::all_queue_names();
        let mut num_due = 0;
        let mut num_reaped = 0;

        for name in names {
            if let Some(queue) = QueueManager::get_opt(&name) {
                if queue.check_reap(now) {
                    num_reaped += 1;
                } else if queue.perform_config_refresh_if_due(now).await {
                    num_due += 1;
                }
            }
        }

        tracing::debug!(
            "refreshed {num_due}, reaped {num_reaped} scheduled queue configs in {:?}",
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
            let mut mgr = MANAGER.lock();

            if !self.queue.is_empty() {
                // Raced with an insert, cannot reap now
                return false;
            }

            tracing::debug!("idling out queue {}", self.name);
            mgr.named.remove(self.name.as_str());
            SCHEDULED_QUEUE_COUNT.set(mgr.named.len() as i64);

            return true;
        }

        false
    }

    async fn perform_config_refresh_if_due(&self, now: Instant) -> bool {
        let due = *self.next_config_refresh.lock();
        if now >= due {
            self.perform_config_refresh().await;
            return true;
        }

        false
    }

    async fn perform_config_refresh(&self) {
        if let Ok(mut config) = load_config().await {
            if let Ok(queue_config) = Queue::call_get_queue_config(&self.name, &mut config).await {
                let strategy = queue_config.strategy;

                self.queue_config.update(queue_config);

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
    fn timeq_insert(&self, msg: Message) -> Result<(), Message> {
        tracing::trace!("timeq_insert {} due={:?}", self.name, msg.get_due());
        match self.queue.insert(msg) {
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
    fn drain_timeq(&self) -> Vec<Message> {
        let msgs = self.queue.drain();
        if !msgs.is_empty() {
            self.metrics().sub(msgs.len() as i64);
            // Wake the maintainer so that it can see that the queue is
            // now empty and decide what it wants to do next.
            self.notify_maintainer.notify_one();
        }
        msgs
    }

    async fn do_rebind(&self, msg: Message, rebind: &Arc<AdminRebindEntry>) {
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
            if let Err(err) = msg.save().await {
                tracing::error!("failed to save msg after rebind: {err:#}");
            }
        }

        let increment_attempts = false;
        let mut delay = None;

        let queue_name = match msg.get_queue_name() {
            Err(err) => {
                tracing::error!("failed to determine queue name for msg: {err:#}");
                if let Err(err) = self.requeue_message(msg, increment_attempts, delay).await {
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
                &*queue_holder
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
            })
            .await;
        }

        if let Err(err) = queue.requeue_message(msg, increment_attempts, delay).await {
            tracing::error!(
                "failed to requeue message to {} after failed rebind: {err:#}",
                queue.name
            );
        }
    }

    #[instrument(skip(self))]
    pub async fn rebind_all(&self, rebind: &Arc<AdminRebindEntry>) {
        let msgs = self.drain_timeq();
        let count = msgs.len();
        if count > 0 {
            for msg in msgs {
                self.do_rebind(msg, rebind).await;
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
            let result = QMAINT_RUNTIME.spawn_non_blocking(
                "bounce_all remove_from_spool".to_string(),
                move || {
                    Ok(async move {
                        for msg in msgs {
                            let id = *msg.id();
                            bounce.log(msg, Some(&name)).await;
                            SpoolManager::remove_from_spool(id).await.ok();
                        }
                    })
                },
            );
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
                source_address: None,
            })
            .await;
            SpoolManager::remove_from_spool(id).await?;
            return Ok(None);
        }
        tracing::trace!("increment_attempts_and_update_delay: delaying {id} by {delay} (num_attempts={num_attempts})");
        msg.delay_by(delay).await?;
        Ok(Some(msg))
    }

    #[instrument(skip(self, msg))]
    pub async fn requeue_message(
        &self,
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
    async fn insert_delayed(&self, msg: Message) -> anyhow::Result<InsertResult> {
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
                            if let Err(err) = self.did_insert_delayed(msg.clone()).await {
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
    async fn force_into_delayed(&self, msg: Message) -> anyhow::Result<()> {
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

    async fn check_message_rate_throttle(&self) -> anyhow::Result<Option<ThrottleResult>> {
        if let Some(throttle) = &self.queue_config.borrow().max_message_rate {
            let result = throttle
                .throttle(format!("schedq-{}-message-rate", self.name))
                .await?;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    fn metrics(&self) -> &ScheduledMetrics {
        self.metrics
            .get_or_init(|| ScheduledMetrics::new(self.name.clone()))
    }

    #[instrument(skip(self, msg))]
    async fn insert_ready(&self, msg: Message) -> anyhow::Result<()> {
        if let Some(result) = self.check_message_rate_throttle().await? {
            if let Some(delay) = result.retry_after {
                tracing::trace!("{} throttled message rate, delay={delay:?}", self.name);
                let delay = chrono::Duration::from_std(delay).unwrap_or(kumo_chrono_helper::MINUTE);
                // We're not using jitter here because the throttle should
                // ideally result in smooth message flow and the jitter will
                // (intentionally) perturb that.
                msg.delay_by(delay).await?;

                self.metrics().delay_due_to_message_rate_throttle().inc();

                return self.force_into_delayed(msg).await;
            }
        }

        let mut config = load_config().await?;
        config
            .async_call_callback(&THROTTLE_INSERT_READY_SIG, msg.clone())
            .await?;
        if let Some(due) = msg.get_due() {
            let now = Utc::now();
            if due > now {
                tracing::trace!(
                    "{}: throttle_insert_ready_queue event throttled message rate, due={due:?}",
                    self.name
                );
                self.metrics().delay_due_to_throttle_insert_ready().inc();

                return self.force_into_delayed(msg).await;
            }
        }

        if let Err(err) = self.insert_ready_impl(msg.clone()).await {
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
                self.metrics().delay_due_to_ready_queue_full().inc();
                self.force_into_delayed(msg).await?;
            }
        }
        Ok(())
    }

    fn get_ready_queue_for_source(&self, source: &str) -> Option<Arc<CachedReadyQueueName>> {
        let mut ready_queue_names = self.ready_queue_names.lock();
        let name = ready_queue_names.get(source)?;

        if self.queue_config.generation() != name.generation || name.name.has_expired() {
            ready_queue_names.remove(source);
            return None;
        }

        Some(Arc::clone(name))
    }

    async fn compute_ready_queue_name(
        &self,
        source: &str,
    ) -> anyhow::Result<Arc<CachedReadyQueueName>> {
        if let Some(entry) = self.get_ready_queue_for_source(source) {
            return Ok(entry);
        }

        let generation = self.queue_config.generation();

        let name =
            ReadyQueueManager::compute_queue_name(&self.name, &self.queue_config, source).await?;

        let cached = Arc::new(CachedReadyQueueName { name, generation });

        self.ready_queue_names
            .lock()
            .insert(source.to_string(), cached.clone());

        Ok(cached)
    }

    #[instrument(skip(self, msg))]
    async fn insert_ready_impl(&self, msg: Message) -> anyhow::Result<()> {
        tracing::trace!("insert_ready {}", msg.id());

        match &self.queue_config.borrow().protocol {
            DeliveryProto::Smtp { .. } | DeliveryProto::Lua { .. } => {
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
                            source_address: None,
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
                            source_address: None,
                        })
                        .await;
                        anyhow::bail!("no non-zero-weighted sources available for {}", self.name);
                    }
                };

                // Hot path: use cached source -> ready queue mapping
                let ready_name = self.compute_ready_queue_name(&egress_source).await?;
                if let Some(site) = ReadyQueueManager::get_by_ready_queue_name(&ready_name.name) {
                    return site.insert(msg).map_err(|_| ReadyQueueFull.into());
                }

                // Miss: compute and establish a new queue
                match ReadyQueueManager::resolve_by_queue_name(
                    &self.name,
                    &self.queue_config,
                    &egress_source,
                    &self.rr.name,
                )
                .await
                {
                    Ok(site) => {
                        return site.insert(msg).map_err(|_| ReadyQueueFull.into());
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
                        })
                        .await;
                        anyhow::bail!("failed to resolve queue {}: {err:#}", self.name);
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
                let result: anyhow::Result<String> = spawn_blocking_on(
                    "write to maildir",
                    {
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
                    },
                    &get_main_runtime(),
                )?
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
                            source_address: None,
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
                            source_address: None,
                        })
                        .await;
                        anyhow::bail!("failed maildir store: {err:#}");
                    }
                }
            }
        }
    }

    #[instrument(fields(self.name), skip(self, msg))]
    pub async fn insert(&self, msg: Message) -> anyhow::Result<()> {
        loop {
            *self.last_change.lock() = Instant::now();

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

                    self.insert_ready(msg.clone()).await?;
                    return Ok(());
                }
            }
        }
    }

    pub fn get_config(&self) -> &ConfigHandle<QueueConfig> {
        &self.queue_config
    }

    async fn run_singleton_wheel() -> anyhow::Result<()> {
        let mut shutdown = ShutdownSubcription::get();

        tracing::debug!("singleton_wheel: starting up");

        async fn reinsert_ready(msg: Message) -> anyhow::Result<()> {
            if !msg.is_meta_loaded() {
                msg.load_meta().await?;
            }
            let queue_name = msg.get_queue_name()?;
            let queue = QueueManager::resolve(&queue_name).await?;

            // Verify that the message is still in the queue
            match &queue.queue {
                QueueStructure::SingletonTimerWheel(q) => {
                    fn remove(q: &StdMutex<HashSet<Message>>, msg: &Message) -> bool {
                        q.lock().remove(msg)
                    }

                    if remove(q, &msg) {
                        queue.metrics().sub(1);
                        queue.insert_ready(msg).await?;
                    }
                }
                _ => {
                    anyhow::bail!("impossible queue strategy");
                }
            }

            Ok(())
        }

        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3)) => {
                    TOTAL_QMAINT_RUNS.inc();

                    fn pop() -> Vec<WeakMessage> {
                        if let PopResult::Items(weak_messages) = SINGLETON_WHEEL.lock().pop() {
                            tracing::debug!("singleton_wheel: popped {} messages", weak_messages.len());
                            weak_messages
                        } else {
                            vec![]
                        }
                    }

                    let mut reinserted = 0;
                    for weak_message in pop() {
                        if let Some(msg) = weak_message.upgrade() {
                            reinserted += 1;
                            if let Err(err) = reinsert_ready(msg).await {
                                tracing::error!("singleton_wheel: reinsert_ready: {err:#}");
                            }
                        }
                    }
                    tracing::debug!("singleton_wheel: done reinserting {reinserted}");

                }
                _ = shutdown.shutting_down() => {
                    tracing::info!("singleton_wheel: stopping");
                    return Ok(());
                }
            }
        }
    }
}

#[must_use]
enum InsertResult {
    Delayed,
    Ready(Message),
}

pub struct QueueManager {
    named: HashMap<String, QueueSlot>,
}

enum QueueSlot {
    Handle(QueueHandle),
    Resolving(Arc<Notify>),
}

enum SlotLease {
    Handle(QueueHandle),
    Resolving(Arc<Notify>),
    MustCreate(Arc<Notify>),
}

async fn queue_meta_maintainer() -> anyhow::Result<()> {
    let activity = Activity::get(format!("Queue Manager Meta Maintainer"))?;
    let mut shutdown = ShutdownSubcription::get();
    shutdown.shutting_down().await;
    loop {
        let names = QueueManager::all_queue_names();
        if names.is_empty() && ReadyQueueManager::number_of_queues() == 0 {
            tracing::debug!("All queues are reaped");
            drop(activity);
            return Ok(());
        }

        for name in names {
            if let Some(queue) = QueueManager::get_opt(&name) {
                for msg in queue.drain_timeq() {
                    Queue::save_if_needed_and_log(&msg).await;
                }
                if queue.queue.is_empty() && ReadyQueueManager::number_of_queues() == 0 {
                    tracing::debug!(
                        "{name}: there are no more queues and the scheduled queue is empty, reaping"
                    );
                    QueueManager::remove(&name);
                }
            }
        }

        tokio::time::sleep(ONE_SECOND).await;
    }
}

impl QueueManager {
    pub fn new() -> Self {
        kumo_server_runtime::get_main_runtime().spawn(queue_meta_maintainer());
        QMAINT_RUNTIME
            .spawn_non_blocking("queue_config_maintainer".to_string(), move || {
                Ok(async move {
                    Queue::queue_config_maintainer().await;
                })
            })
            .expect("failed to spawn queue_config_maintainer");
        Self {
            named: HashMap::new(),
        }
    }

    /// Insert message into a queue named `name`.
    #[instrument(skip(msg))]
    pub async fn insert(name: &str, msg: Message) -> anyhow::Result<()> {
        tracing::trace!("QueueManager::insert");
        let timer = RESOLVE_LATENCY.start_timer();
        let entry = Self::resolve(name).await?;
        timer.stop_and_record();

        let _timer = INSERT_LATENCY.start_timer();
        entry.insert(msg).await
    }

    fn resolve_lease(name: &str) -> SlotLease {
        let mut mgr = MANAGER.lock();
        match mgr.named.get(name) {
            Some(QueueSlot::Handle(e)) => SlotLease::Handle(Arc::clone(e)),
            Some(QueueSlot::Resolving(notify)) => SlotLease::Resolving(notify.clone()),
            None => {
                // Insert a Resolving slot, so that other actors know to wait
                let notify = Arc::new(Notify::new());
                mgr.named
                    .insert(name.to_string(), QueueSlot::Resolving(notify.clone()));
                SCHEDULED_QUEUE_COUNT.set(mgr.named.len() as i64);

                SlotLease::MustCreate(notify)
            }
        }
    }

    /// Resolve a scheduled queue name to a handle,
    /// returning a pre-existing handle if it is already known.
    #[instrument]
    pub async fn resolve(name: &str) -> anyhow::Result<QueueHandle> {
        match Self::resolve_lease(name) {
            SlotLease::Handle(e) => Ok(e),
            SlotLease::Resolving(notify) => {
                notify.notified().await;
                Self::get_opt(name)
                    .ok_or_else(|| anyhow::anyhow!("other actor failed to resolve {name}"))
            }
            SlotLease::MustCreate(notify) => {
                let result = Queue::new(name.to_string()).await;
                let mut mgr = MANAGER.lock();
                // Wake up any other waiters, regardless of the outcome
                notify.notify_waiters();

                match result {
                    Ok(entry) => {
                        // Success! move from Resolving -> Handle
                        mgr.named
                            .insert(name.to_string(), QueueSlot::Handle(entry.clone()));
                        SCHEDULED_QUEUE_COUNT.set(mgr.named.len() as i64);
                        Ok(entry)
                    }
                    Err(err) => {
                        // Failed! remove the Resolving slot
                        mgr.named.remove(name);
                        SCHEDULED_QUEUE_COUNT.set(mgr.named.len() as i64);
                        Err(err)
                    }
                }
            }
        }
    }

    pub fn get_opt(name: &str) -> Option<QueueHandle> {
        let mgr = MANAGER.lock();
        match mgr.named.get(name)? {
            QueueSlot::Handle(h) => Some(h.clone()),
            QueueSlot::Resolving(_) => None,
        }
    }

    pub fn all_queue_names() -> Vec<String> {
        let mgr = MANAGER.lock();
        mgr.named.keys().map(|s| s.to_string()).collect()
    }

    /// Coupled with Queue::check_reap!
    pub fn remove(name: &str) {
        let mut mgr = MANAGER.lock();
        mgr.named.remove(name);
        SCHEDULED_QUEUE_COUNT.set(mgr.named.len() as i64);
    }
}

#[instrument(skip(q))]
async fn maintain_named_queue(q: &QueueHandle) -> anyhow::Result<()> {
    let mut shutdown = ShutdownSubcription::get();
    let mut memory = kumo_server_memory::subscribe_to_memory_status_changes();
    let mut next_item_due = Instant::now() + ONE_DAY;

    loop {
        let sleeping = Instant::now();
        let reason = tokio::select! {
            _ = tokio::time::sleep_until(next_item_due.into()) => {"due"}
            _ = shutdown.shutting_down() => {"shutting_down"}
            _ = memory.changed() => {"memory"}
            _ = q.notify_maintainer.notified() => {"notified"}
        };

        TOTAL_QMAINT_RUNS.inc();

        {
            tracing::debug!(
                "maintaining {} {:?} which has {} entries. wakeup after {:?} reason={reason}",
                q.name,
                q.queue.strategy(),
                q.queue.len(),
                sleeping.elapsed(),
            );

            if let Some(b) = AdminBounceEntry::get_for_queue_name(&q.name) {
                q.bounce_all(&b).await;
            }

            if q.activity.is_shutting_down() {
                for msg in q.drain_timeq() {
                    Queue::save_if_needed_and_log(&msg).await;
                    drop(msg);
                }

                // Bow out and let the queue_meta_maintainer finish up
                return Ok(());
            }

            let (messages, next_due_in) = q.queue.pop();

            let now = Instant::now();

            next_item_due = if q.queue.is_timer_wheel() {
                // For a timer wheel, we need to (fairly consistently) tick it
                // over in order to promote things to the ready queue.
                // We do this based on the retry duration; the product default
                // is a 20m retry duration for which we want to tick once per
                // minute.
                // For shorter intervals we scale this accordingly.
                // To avoid very excessively wakeups for very short or very
                // long intervals, we clamp to between 1s and 1m.

                debug_assert!(
                    next_due_in.is_none(),
                    "next_due_in should never be populated for timerwheel"
                );

                let queue_config = q.queue_config.borrow();
                now + queue_config.timerwheel_tick_interval.unwrap_or(
                    (queue_config.retry_interval / 20)
                        .max(ONE_SECOND)
                        .min(ONE_MINUTE),
                )
            } else {
                now + next_due_in.unwrap_or(ONE_DAY)
            };

            if !messages.is_empty() {
                q.metrics().sub(messages.len() as i64);
                tracing::debug!("{} {} msgs are now ready", q.name, messages.len());

                for msg in messages {
                    q.insert_ready(msg).await?;
                }
            }
        }
    }
}
