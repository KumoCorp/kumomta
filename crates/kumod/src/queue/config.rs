use crate::queue::delivery_proto::DeliveryProto;
use crate::queue::strategy::QueueStrategy;
use kumo_api_types::egress_path::{ConfigRefreshStrategy, MemoryReductionPolicy};
use mlua::prelude::*;
use mlua::UserDataMethods;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use throttle::ThrottleSpec;

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);
const ONE_MINUTE: Duration = Duration::from_secs(60);

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

    #[serde(default)]
    pub shrink_policy: Vec<QueueShrinkPolicy>,
}

#[derive(Deserialize, Serialize, Debug, Clone, FromLua)]
#[serde(deny_unknown_fields)]
pub struct QueueShrinkPolicy {
    #[serde(with = "duration_serde")]
    pub interval: Duration,
    pub policy: MemoryReductionPolicy,
}

impl LuaUserData for QueueConfig {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        config::impl_pairs_and_index(methods);
    }
}

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
            shrink_policy: Default::default(),
        }
    }
}

/// The largest seconds value that can be passed to chrono::Duration::try_seconds
/// before it returns None.
const MAX_CHRONO_SECONDS: i64 = i64::MAX / 1_000;

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
            elapsed += delay;
            num_attempts += 1;
        }
    }

    pub fn delay_for_attempt(&self, attempt: u16) -> chrono::Duration {
        let delay = self
            .retry_interval
            .as_secs()
            .saturating_mul(2u64.saturating_pow(attempt as u32));

        let delay = match self.max_retry_interval.map(|d| d.as_secs()) {
            None => delay,
            Some(limit) => delay.min(limit),
        };

        chrono::Duration::try_seconds((delay as i64).min(MAX_CHRONO_SECONDS))
            .expect("seconds to always be <= MAX_CHRONO_SECONDS")
    }

    /// Compute the delay from the current point in time until
    /// the next due time for a hypothetical message whose
    /// num_attempts and age (the duration
    /// since it was created, until the current point in time
    /// when this function is being called) are given.
    ///
    /// age therefore implies the `now` value.
    ///
    /// Returns Some(delay) if the delay from now is within the
    /// max_age defined by the queue config, or None if the
    /// overall delay (since creation of the message) exceeds
    /// the allowed max_age.
    pub fn compute_delay_based_on_age(
        &self,
        num_attempts: u16,
        age: chrono::Duration,
    ) -> Option<chrono::Duration> {
        let max_age = self.get_max_age();
        if age >= max_age {
            return None;
        }

        // Compute the delay from the creation time of the message
        // based on the number of attempts
        let overall_delay: i64 = (1..num_attempts)
            .map(|i| self.delay_for_attempt(i).num_seconds())
            .sum();
        let overall_delay = chrono::Duration::try_seconds(overall_delay.min(MAX_CHRONO_SECONDS))
            .expect("seconds to always be <= MAX_CHRONO_SECONDS");

        if overall_delay >= max_age {
            // It would be outside the permitted age
            None
        } else {
            Some(
                // adjust to be relative to the `now` time implied by `age`,
                // and ensure that it cannot be negative
                overall_delay
                    .checked_sub(&age)
                    .unwrap_or_else(chrono::Duration::zero)
                    .max(chrono::Duration::zero()),
            )
        }
    }

    /// Compute the delay from the current point in time until
    /// the next due time for a hypothetical message whose
    /// num_attempts and age (the duration
    /// since it was created, until the current point in time
    /// when this function is being called) are given.
    ///
    /// age therefore implies the `now` value.
    ///
    /// This function does not care about max_age.
    pub fn compute_delay_based_on_age_ignoring_max_age(
        &self,
        num_attempts: u16,
        age: chrono::Duration,
    ) -> chrono::Duration {
        let overall_delay: i64 = (1..num_attempts)
            .map(|i| self.delay_for_attempt(i).num_seconds())
            .sum();
        let overall_delay = chrono::Duration::try_seconds(overall_delay.min(MAX_CHRONO_SECONDS))
            .expect("seconds to always be <= MAX_SECONDS");

        // adjust to be relative to the `now` time implied by `age`,
        // and ensure that it cannot be negative
        overall_delay
            .checked_sub(&age)
            .unwrap_or_else(chrono::Duration::zero)
            .max(chrono::Duration::zero())
    }
}
