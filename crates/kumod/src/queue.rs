use crate::egress_source::{EgressPool, EgressPoolRoundRobin, RoundRobinResult};
use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::http_server::admin_rebind_v1::AdminRebindEntry;
use crate::http_server::admin_suspend_v1::AdminSuspendEntry;
use crate::http_server::inject_v1::{make_generate_queue_config, GENERATOR_QUEUE_NAME};
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::lua_deliver::LuaDeliveryProtocol;
use crate::metrics_helper::{
    BorrowedProviderAndPoolKey, BorrowedProviderKey, ProviderAndPoolKeyTrait, ProviderKeyTrait,
    QUEUED_COUNT_GAUGE_BY_PROVIDER, QUEUED_COUNT_GAUGE_BY_PROVIDER_AND_POOL,
};
use crate::ready_queue::ReadyQueueManager;
use crate::smtp_dispatcher::SmtpProtocol;
use crate::smtp_server::RejectError;
use crate::spool::SpoolManager;
use anyhow::Context;
use chrono::{DateTime, Utc};
use config::epoch::{get_current_epoch, ConfigEpoch};
use config::{load_config, CallbackSignature, LuaConfig};
use crossbeam_skiplist::SkipSet;
use kumo_api_types::egress_path::ConfigRefreshStrategy;
use kumo_prometheus::{counter_bundle, label_key, AtomicCounter, PruningCounterRegistry};
use kumo_server_common::config_handle::ConfigHandle;
use kumo_server_lifecycle::{is_shutting_down, Activity, ShutdownSubcription};
use kumo_server_runtime::{get_main_runtime, spawn, spawn_blocking_on, Runtime};
use message::message::{QueueNameComponents, WeakMessage};
use message::Message;
use mlua::prelude::*;
use parking_lot::FairMutex as StdMutex;
use prometheus::{Histogram, IntCounter, IntGauge};
use rfc5321::{EnhancedStatusCode, Response};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Once, OnceLock};
use std::time::{Duration, Instant};
use thiserror::Error;
use throttle::{ThrottleResult, ThrottleSpec};
use timeq::{PopResult, TimeQ, TimerError};
use tokio::sync::Notify;
use tracing::instrument;

static MANAGER: LazyLock<StdMutex<QueueManager>> =
    LazyLock::new(|| StdMutex::new(QueueManager::new()));
static SCHEDULED_QUEUE_COUNT: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "scheduled_queue_count",
        "how many scheduled queues are tracked by the QueueManager"
    )
    .unwrap()
});
static QMAINT_COUNT: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "scheduled_queue_maintainer_count",
        "how many scheduled queues have active maintainer tasks"
    )
    .unwrap()
});
static TOTAL_QMAINT_RUNS: LazyLock<IntCounter> = LazyLock::new(|| {
    prometheus::register_int_counter!(
        "total_qmaint_runs",
        "total number of times a scheduled queue maintainer was run"
    )
    .unwrap()
});

pub static QMAINT_RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("qmaint", |cpus| cpus / 4, &QMAINT_THREADS).unwrap());
pub static GET_Q_CONFIG_SIG: LazyLock<
    CallbackSignature<
        (
            &'static str,
            Option<&'static str>,
            Option<&'static str>,
            Option<&'static str>,
        ),
        QueueConfig,
    >,
> = LazyLock::new(|| CallbackSignature::new_with_multiple("get_queue_config"));
pub static THROTTLE_INSERT_READY_SIG: LazyLock<CallbackSignature<Message, ()>> =
    LazyLock::new(|| CallbackSignature::new_with_multiple("throttle_insert_ready_queue"));
static REBIND_MESSAGE_SIG: LazyLock<CallbackSignature<(Message, HashMap<String, String>), ()>> =
    LazyLock::new(|| CallbackSignature::new("rebind_message"));
pub static REQUEUE_MESSAGE_SIG: LazyLock<CallbackSignature<(Message, String), ()>> =
    LazyLock::new(|| CallbackSignature::new_with_multiple("requeue_message"));

pub static SINGLETON_WHEEL: LazyLock<Arc<StdMutex<TimeQ<WeakMessage>>>> =
    LazyLock::new(|| Arc::new(StdMutex::new(TimeQ::new())));

static TOTAL_DELAY_GAUGE: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "scheduled_count_total",
        "total number of messages across all scheduled queues",
    )
    .unwrap()
});

label_key! {
    pub struct QueueKey {
        pub queue: String,
    }
}
label_key! {
    pub struct TenantKey {
        pub tenant: String,
    }
}
label_key! {
    pub struct TenantCampaignKey {
        pub tenant: String,
        pub campaign: String,
    }
}
label_key! {
    pub struct DomainKey{
        pub domain: String,
    }
}

static DELAY_GAUGE: LazyLock<PruningCounterRegistry<QueueKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register_gauge(
        "scheduled_count",
        "number of messages in the scheduled queue",
    )
});
static TENANT_GAUGE: LazyLock<PruningCounterRegistry<TenantKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register_gauge(
        "scheduled_by_tenant",
        "number of messages in the scheduled queue for a specific tenant",
    )
});

static TENANT_CAMPAIGN_GAUGE: LazyLock<PruningCounterRegistry<TenantCampaignKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register_gauge(
        "scheduled_by_tenant_campaign",
        "number of messages in the scheduled queue for a specific tenant and campaign combination",
    )
    });

static DOMAIN_GAUGE: LazyLock<PruningCounterRegistry<DomainKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register_gauge(
        "scheduled_by_domain",
        "number of messages in the scheduled queue for a specific domain",
    )
});

static DELAY_DUE_TO_READY_QUEUE_FULL_COUNTER: LazyLock<PruningCounterRegistry<QueueKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "delayed_due_to_ready_queue_full",
            "number of times a message was delayed due to the corresponding ready queue being full",
        )
    });

static DELAY_DUE_TO_MESSAGE_RATE_THROTTLE_COUNTER: LazyLock<PruningCounterRegistry<QueueKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "delayed_due_to_message_rate_throttle",
            "number of times a message was delayed due to max_message_rate",
        )
    });
static DELAY_DUE_TO_THROTTLE_INSERT_READY_COUNTER: LazyLock<PruningCounterRegistry<QueueKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "delayed_due_to_throttle_insert_ready",
            "number of times a message was delayed due throttle_insert_ready_queue event",
        )
    });
static RESOLVE_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "queue_resolve_latency",
        "latency of QueueManager::resolve operations",
    )
    .unwrap()
});
static INSERT_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
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

counter_bundle! {
    pub struct ScheduledCountBundle {
        pub delay_gauge: AtomicCounter,
        pub queued_by_provider: AtomicCounter,
        pub queued_by_provider_and_pool: AtomicCounter,
        pub by_domain: AtomicCounter,
    }
}

struct ScheduledMetrics {
    name: Arc<String>,
    scheduled: ScheduledCountBundle,
    by_tenant: Option<AtomicCounter>,
    by_tenant_campaign: Option<AtomicCounter>,
    delay_due_to_message_rate_throttle: OnceLock<AtomicCounter>,
    delay_due_to_throttle_insert_ready: OnceLock<AtomicCounter>,
    delay_due_to_ready_queue_full: OnceLock<AtomicCounter>,
}

impl ScheduledMetrics {
    pub fn new(name: Arc<String>, pool: &str, site: &str, provider_name: &Option<String>) -> Self {
        let components = QueueNameComponents::parse(&name);

        let queue_key = BorrowedQueueKey {
            queue: name.as_str(),
        };
        let domain_key = BorrowedDomainKey {
            domain: components.domain,
        };

        let by_domain = DOMAIN_GAUGE.get_or_create(&domain_key as &dyn DomainKeyTrait);

        let by_tenant = components.tenant.map(|tenant| {
            let tenant_key = BorrowedTenantKey { tenant };
            TENANT_GAUGE.get_or_create(&tenant_key as &dyn TenantKeyTrait)
        });
        let by_tenant_campaign = match &components.campaign {
            Some(campaign) => {
                let key = BorrowedTenantCampaignKey {
                    tenant: components.tenant.as_ref().map(|s| s.as_ref()).unwrap_or(""),
                    campaign,
                };
                Some(TENANT_CAMPAIGN_GAUGE.get_or_create(&key as &dyn TenantCampaignKeyTrait))
            }
            None => None,
        };

        let provider_key = match provider_name {
            Some(provider) => BorrowedProviderKey { provider },
            None => BorrowedProviderKey { provider: site },
        };
        let provider_pool_key = match provider_name {
            Some(provider) => BorrowedProviderAndPoolKey { provider, pool },
            None => BorrowedProviderAndPoolKey {
                provider: site,
                pool,
            },
        };

        let scheduled = ScheduledCountBundle {
            delay_gauge: DELAY_GAUGE.get_or_create(&queue_key as &dyn QueueKeyTrait),
            queued_by_provider: QUEUED_COUNT_GAUGE_BY_PROVIDER
                .get_or_create(&provider_key as &dyn ProviderKeyTrait),
            queued_by_provider_and_pool: QUEUED_COUNT_GAUGE_BY_PROVIDER_AND_POOL
                .get_or_create(&provider_pool_key as &dyn ProviderAndPoolKeyTrait),
            by_domain,
        };

        Self {
            name,
            by_tenant,
            by_tenant_campaign,
            scheduled,
            delay_due_to_message_rate_throttle: OnceLock::new(),
            delay_due_to_throttle_insert_ready: OnceLock::new(),
            delay_due_to_ready_queue_full: OnceLock::new(),
        }
    }

    pub fn delay_due_to_message_rate_throttle(&self) -> &AtomicCounter {
        self.delay_due_to_message_rate_throttle.get_or_init(|| {
            let key = BorrowedQueueKey {
                queue: self.name.as_str(),
            };
            DELAY_DUE_TO_MESSAGE_RATE_THROTTLE_COUNTER.get_or_create(&key as &dyn QueueKeyTrait)
        })
    }
    pub fn delay_due_to_throttle_insert_ready(&self) -> &AtomicCounter {
        self.delay_due_to_throttle_insert_ready.get_or_init(|| {
            let key = BorrowedQueueKey {
                queue: self.name.as_str(),
            };
            DELAY_DUE_TO_THROTTLE_INSERT_READY_COUNTER.get_or_create(&key as &dyn QueueKeyTrait)
        })
    }
    pub fn delay_due_to_ready_queue_full(&self) -> &AtomicCounter {
        self.delay_due_to_ready_queue_full.get_or_init(|| {
            let key = BorrowedQueueKey {
                queue: self.name.as_str(),
            };

            DELAY_DUE_TO_READY_QUEUE_FULL_COUNTER.get_or_create(&key as &dyn QueueKeyTrait)
        })
    }

    pub fn inc(&self) {
        TOTAL_DELAY_GAUGE.inc();
        self.scheduled.inc();
        self.by_tenant.as_ref().map(|m| m.inc());
        self.by_tenant_campaign.as_ref().map(|m| m.inc());
    }

    pub fn sub(&self, amount: usize) {
        TOTAL_DELAY_GAUGE.sub(amount as i64);
        self.scheduled.sub(amount);
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
    HttpInjectionGenerator,
    Null,
}

impl DeliveryProto {
    pub fn metrics_protocol_name(&self) -> &'static str {
        match self {
            Self::Smtp { .. } => "smtp_client",
            Self::Maildir { .. } => "maildir",
            Self::Lua { .. } => "lua",
            Self::HttpInjectionGenerator { .. } => "httpinject",
            Self::Null { .. } => "null",
        }
    }

    pub fn ready_queue_name(&self) -> String {
        let proto_name = self.metrics_protocol_name();
        match self {
            Self::Smtp { .. } | Self::Null => proto_name.to_string(),
            Self::Maildir { maildir_path } => format!("{proto_name}:{}", maildir_path.display()),
            Self::Lua { custom_lua } => format!("{proto_name}:{}", custom_lua.constructor),
            Self::HttpInjectionGenerator => format!("{proto_name}:generator"),
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
    TimerWheel,
    SkipList,
    #[default]
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

    #[serde(default)]
    pub refresh_strategy: ConfigRefreshStrategy,

    /// Specify an explicit provider name that should apply to this
    /// queue. The provider name will be used when computing metrics
    /// rollups by provider. If omitted, then a provider derived
    /// from the site_name, which is in turn derived from the
    /// routing_domain for this queue, will be used instead.
    #[serde(default)]
    pub provider_name: Option<String>,
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
            refresh_strategy: ConfigRefreshStrategy::default(),
            provider_name: None,
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
                    should_notify: if due.is_none() { true } else { now_due < due },
                }
            }
            Self::SingletonTimerWheel(q) => {
                match SINGLETON_WHEEL.lock().insert(msg.weak()) {
                    Ok(()) => {
                        q.lock().insert(msg);
                        STARTED_SINGLETON_WHEEL.call_once(|| {
                            QMAINT_RUNTIME
                                .spawn("singleton_wheel".to_string(), async move {
                                    if let Err(err) = Queue::run_singleton_wheel().await {
                                        tracing::error!("run_singleton_wheel: {err:#}");
                                    }
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
    metrics: OnceLock<ScheduledMetrics>,
    activity: Activity,
    rr: EgressPoolRoundRobin,
    next_config_refresh: StdMutex<Instant>,
    warned_strategy_change: AtomicBool,
    config_epoch: StdMutex<ConfigEpoch>,
    site_name: String,
}

impl Queue {
    async fn call_get_queue_config(
        name: &str,
        config: &mut LuaConfig,
    ) -> anyhow::Result<QueueConfig> {
        if name == GENERATOR_QUEUE_NAME {
            return make_generate_queue_config();
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
        let rr = EgressPoolRoundRobin::new(&pool);

        let activity = Activity::get(format!("Queue {name}"))?;
        let strategy = queue_config.strategy;
        let next_config_refresh = StdMutex::new(Instant::now() + queue_config.refresh_interval);

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
            last_change: StdMutex::new(Instant::now()),
            queue_config,
            notify_maintainer: Arc::new(Notify::new()),
            metrics: OnceLock::new(),
            activity,
            rr,
            next_config_refresh,
            warned_strategy_change: AtomicBool::new(false),
            config_epoch: StdMutex::new(epoch),
            site_name,
        });

        if !matches!(strategy, QueueStrategy::SingletonTimerWheel) {
            Self::spawn_queue_maintainer(&handle)?;
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

    async fn queue_config_maintainer() {
        let mut shutdown = ShutdownSubcription::get();
        let mut epoch_subscriber = config::epoch::subscribe();
        let mut last_epoch = epoch_subscriber.borrow_and_update().clone();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    Self::check_config_refresh(&last_epoch, false).await;
                }
                _ = epoch_subscriber.changed() => {
                    let this_epoch = epoch_subscriber.borrow_and_update().clone();
                    tracing::debug!("queue_config_maintainer: epoch changed from \
                                     {last_epoch:?} -> {this_epoch:?}");
                    last_epoch = this_epoch.clone();
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

    fn get_config_epoch(&self) -> ConfigEpoch {
        self.config_epoch.lock().clone()
    }

    fn set_config_epoch(&self, epoch: &ConfigEpoch) {
        *self.config_epoch.lock() = epoch.clone();
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
            self.metrics().sub(msgs.len());
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

        let mut delay = None;

        let queue_name = match msg.get_queue_name() {
            Err(err) => {
                tracing::error!("failed to determine queue name for msg: {err:#}");
                if let Err(err) = self
                    .requeue_message_internal(msg, IncrementAttempts::No, delay)
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
                provider: None,
                session_id: None,
            })
            .await;
        }

        if let Err(err) = queue
            .requeue_message_internal(msg, IncrementAttempts::No, delay)
            .await
        {
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
                provider: self.queue_config.borrow().provider_name.as_deref(),
                session_id: None,
            })
            .await;
            SpoolManager::remove_from_spool(id).await?;
            return Ok(None);
        }
        tracing::trace!("increment_attempts_and_update_delay: delaying {id} by {delay} (num_attempts={num_attempts})");
        msg.delay_by(delay).await?;
        Ok(Some(msg))
    }

    /// Performs the raw re-insertion of a message into a scheduled queue.
    /// The requeue_message event is NOT called by this function.
    #[instrument(skip(self, msg))]
    async fn requeue_message_internal(
        &self,
        msg: Message,
        increment_attempts: IncrementAttempts,
        delay: Option<chrono::Duration>,
    ) -> anyhow::Result<()> {
        if increment_attempts == IncrementAttempts::Yes {
            match self.increment_attempts_and_update_delay(msg).await? {
                Some(msg) => {
                    return self.insert(msg).await;
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
                })
                .await;
                SpoolManager::remove_from_spool(id).await?;
                return Ok(());
            }
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
    async fn insert_ready(&self, msg: Message) -> anyhow::Result<()> {
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
            })
            .await;

            Box::pin(QueueManager::requeue_message(
                msg,
                IncrementAttempts::Yes,
                None,
                response,
            ))
            .await?;

            return Ok(());
        }

        if let Some(result) = self.check_message_rate_throttle().await? {
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
                ))
                .await?;

                self.metrics().delay_due_to_message_rate_throttle().inc();

                return Ok(());
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
                ))
                .await?;
                return Ok(());
            }
        }

        if let Err(err) = self.insert_ready_impl(msg.clone()).await {
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
                ))
                .await?;
            } else {
                // Queue is full; try again shortly
                self.metrics().delay_due_to_ready_queue_full().inc();
                self.force_into_delayed(msg).await?;
            }
        }
        Ok(())
    }

    #[instrument(skip(self, msg))]
    async fn insert_ready_impl(&self, msg: Message) -> anyhow::Result<()> {
        tracing::trace!("insert_ready {}", msg.id());

        match &self.queue_config.borrow().protocol {
            DeliveryProto::Smtp { .. }
            | DeliveryProto::Lua { .. }
            | DeliveryProto::HttpInjectionGenerator => {
                let (egress_source, ready_name) = match self
                    .rr
                    .next(&self.name, &self.queue_config)
                    .await
                {
                    RoundRobinResult::Source {
                        name,
                        ready_queue_name,
                    } => (name, ready_queue_name),
                    RoundRobinResult::Delay(_duration) => {
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
                            provider: self.queue_config.borrow().provider_name.as_deref(),
                            session_id: None,
                        })
                        .await;
                        anyhow::bail!("all possible sources for {} are suspended", self.name);
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
                            provider: self.queue_config.borrow().provider_name.as_deref(),
                            session_id: None,
                        })
                        .await;
                        anyhow::bail!("no non-zero-weighted sources available for {}", self.name);
                    }
                };

                // Hot path: use cached source -> ready queue mapping
                if let Some(ready_name) = &ready_name {
                    if let Some(site) = ReadyQueueManager::get_by_ready_queue_name(&ready_name.name)
                    {
                        return site.insert(msg).await.map_err(|_| ReadyQueueFull.into());
                    }
                }

                // Miss: compute and establish a new queue
                match ReadyQueueManager::resolve_by_queue_name(
                    &self.name,
                    &self.queue_config,
                    &egress_source,
                    &self.rr.name,
                    self.get_config_epoch(),
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
                        })
                        .await;
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
                            provider: None,
                            session_id: None,
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
                            provider: None,
                            session_id: None,
                        })
                        .await;
                        anyhow::bail!("failed maildir store: {err:#}");
                    }
                }
            }
        }
    }

    /// Insert a newly received, or freshly loaded from spool, message
    /// into this queue
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
            // Use get_opt rather than resolve here. If the queue is not currently
            // tracked in the QueueManager then this message cannot possibly belong
            // to it. Using resolve would have the side effect of creating an empty
            // queue for it, which will then age out later. It's a waste to do that,
            // so we just check and skip.
            let queue = QueueManager::get_opt(&queue_name)
                .ok_or_else(|| anyhow::anyhow!("no scheduled queue"))?;

            if let Some(b) = AdminBounceEntry::get_for_queue_name(&queue.name) {
                // Note that this will cause the msg to be removed from the
                // queue so the remove() check below will return false
                queue.bounce_all(&b).await;
            }

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

                    fn pop() -> (Vec<WeakMessage>, usize) {
                        let mut wheel = SINGLETON_WHEEL.lock();

                        let msgs = if let PopResult::Items(weak_messages) = wheel.pop() {
                            tracing::debug!("singleton_wheel: popped {} messages", weak_messages.len());
                            weak_messages
                        } else {
                            vec![]
                        };

                        (msgs, wheel.len())
                    }

                    let mut reinserted = 0;
                    let (msgs, len) = pop();
                    for weak_message in msgs {
                        if let Some(msg) = weak_message.upgrade() {
                            reinserted += 1;
                            if let Err(err) = reinsert_ready(msg).await {
                                tracing::error!("singleton_wheel: reinsert_ready: {err:#}");
                            }
                        }
                    }
                    tracing::debug!("singleton_wheel: done reinserting {reinserted}. total scheduled={len}");

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
            .spawn("queue_config_maintainer".to_string(), async move {
                Queue::queue_config_maintainer().await;
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

    /// Insert message into a queue named `name`, unwinding it in the case
    /// of error. Unwinding here means that:
    ///
    ///  * The message is removed from the spool
    ///  * An internal Bounce is generated in the disposition logs
    ///
    /// This is a wrapper around QueueManager::insert that will remove the
    /// message from the spool and generate a Bounce entry.
    ///
    /// It is intended to be called at the point of ingress, during reception,
    /// and not as a general purpose loader (eg: most definitely NOT during
    /// spool enumeration, where it would have the consequence of deleting
    /// the spool on startup if there was a config issue!).
    #[instrument(skip(msg))]
    pub async fn insert_or_unwind(
        name: &str,
        msg: Message,
        spool_was_deferred: bool,
    ) -> anyhow::Result<()> {
        match Self::insert(name, msg.clone()).await {
            Ok(()) => Ok(()),
            Err(err) => {
                // Well, this sucks. The likely cause is an error in the
                // lua behind either get_queue_config or get_egress_path_config.
                // Since we spooled the message, we need to unwind that before
                // we report the failure back to the user.
                // We cannot just accept the message and continue because
                // we failed to resolve the configuration for it: the message
                // won't go anywhere and we thus cannot accept responsibility
                // from the injector

                // Note that we try to remove from the spool even if we know
                // that spool_was_deferred, because we don't know if someone
                // called msg:save() in some lua code, or if some other logic
                // may be have decided to spool it anyway.
                if let Err(err) = SpoolManager::remove_from_spool(*msg.id()).await {
                    // Note that, at the time of writing this comment,
                    // SpoolManager::remove_from_spool never returns an error.
                    // But let's add some logic here to surface one if it
                    // starts to do so in the future.
                    if !spool_was_deferred {
                        tracing::error!("remove_from_spool({}) failed: {err:#}", msg.id());
                    }
                }

                // Since the caller just logged a Reception, we should now log
                // a Bounce so that the logs reflect that we aren't going
                // to send this message and we don't leave someone scratching
                // their head about it.
                log_disposition(LogDisposition {
                    kind: RecordType::Bounce,
                    msg: msg.clone(),
                    site: "",
                    peer_address: None,
                    response: Response {
                        code: 500,
                        enhanced_code: None,
                        command: None,
                        content: format!(
                        "KumoMTA internal: QueueManager::insert failed during reception: {err:#}"
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
                })
                .await;

                Err(err)
            }
        }
    }

    /// Re-insert message into the queue subsystem, likely the scheduled queue,
    /// after first calling out to the requeue_message event handler, which
    /// gives the user the opportunity to rebind or do other things to the
    /// message before we put it back into the queues.
    #[instrument(skip(msg))]
    pub async fn requeue_message(
        msg: Message,
        mut increment_attempts: IncrementAttempts,
        mut delay: Option<chrono::Duration>,
        response: Response,
    ) -> anyhow::Result<()> {
        if !msg.is_meta_loaded() {
            msg.load_meta().await?;
        }
        let mut queue_name = msg.get_queue_name()?;

        match load_config().await {
            Ok(mut config) => {
                let result: anyhow::Result<()> = config
                    .async_call_callback(
                        &REQUEUE_MESSAGE_SIG,
                        (msg.clone(), response.to_single_line()),
                    )
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
                            increment_attempts = IncrementAttempts::No;

                            // Avoid adding jitter as part of the queue change
                            delay = Some(chrono::Duration::zero());
                            // and ensure that the message is due now
                            msg.set_due(None).await?;

                            // and use the new queue name
                            queue_name = queue_name_after;
                        }
                    }
                    Err(err) => {
                        // If they did a kumo.reject() in the handler, translate that
                        // into a Bounce. We do this even if they used a 4xx code; it
                        // only makes sense to map it to a Bounce rather than a
                        // TransientFailure because we already just had a TransientFailure.
                        if let Some(rej) = RejectError::from_anyhow(&err) {
                            log_disposition(LogDisposition {
                                kind: RecordType::Bounce,
                                msg: msg.clone(),
                                // There is no site because this was a policy bounce
                                // triggered in an event handler
                                site: "",
                                peer_address: None,
                                response: Response {
                                    code: rej.code,
                                    enhanced_code: None,
                                    content: rej.message,
                                    command: None,
                                },
                                egress_pool: None,
                                egress_source: None,
                                relay_disposition: None,
                                delivery_protocol: None,
                                tls_info: None,
                                source_address: None,
                                provider: None,
                                session_id: None,
                            })
                            .await;
                            SpoolManager::remove_from_spool(*msg.id()).await.ok();
                            return Ok(());
                        }

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

        let queue = QueueManager::resolve(&queue_name).await?;
        queue
            .requeue_message_internal(msg, increment_attempts, delay)
            .await
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
    let mut next_item_due = Instant::now();

    loop {
        let sleeping = Instant::now();
        let reason = tokio::select! {
            _ = tokio::time::sleep_until(next_item_due.into()) => {"due"}
            _ = shutdown.shutting_down() => {"shutting_down"}
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
                q.metrics().sub(messages.len());
                tracing::debug!("{} {} msgs are now ready", q.name, messages.len());

                // There can sometimes be a small (eg: 20ms or so) discrepancy
                // between what the time wheel considers to be ready and what
                // the precise due time of the individual messages shows as
                // their due time.
                // That is expected and fine, however: we want to ensure that
                // the actual time is after the due time of this batch of
                // messages so that the logic after THROTTLE_INSERT_READY_SIG
                // doesn't think that the event handler has explicitly delayed
                // the messages and pushes them into the next retry window.
                // This loop accumulates the longest delay from the batch
                // and sleeps until we are past it.
                // An alternative approach to avoiding that confusion might
                // be to call msg.set_due(None), but there is some additional
                // logic in that method that inspects and manipulates scheduling
                // constraints, so it feels slightly better just wait those
                // few milliseconds here than to trigger more work over there.
                let now = Utc::now();
                let mut delay = Duration::from_secs(0);
                for msg in &messages {
                    if let Some(due) = msg.get_due() {
                        if let Ok(delta) = (due - now).to_std() {
                            delay = delay.max(delta);
                        }
                    }
                }
                tokio::time::sleep(delay).await;

                for msg in messages {
                    q.insert_ready(msg).await?;
                }
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IncrementAttempts {
    No,
    Yes,
}
