use crate::metrics_helper::{
    BorrowedProviderAndPoolKey, BorrowedProviderKey, ProviderAndPoolKeyTrait, ProviderKeyTrait,
    QUEUED_COUNT_GAUGE_BY_PROVIDER, QUEUED_COUNT_GAUGE_BY_PROVIDER_AND_POOL,
};
use kumo_prometheus::{counter_bundle, declare_metric, label_key, AtomicCounter};
use message::queue_name::QueueNameComponents;
use std::sync::{Arc, OnceLock};

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

declare_metric! {
/// number of times a message was delayed due to the corresponding ready queue being full
static DELAY_DUE_TO_READY_QUEUE_FULL_COUNTER: PruningCounterRegistry<QueueKey>(
            "delayed_due_to_ready_queue_full");
}

declare_metric! {
/// number of times a message was delayed due to max_message_rate
static DELAY_DUE_TO_MESSAGE_RATE_THROTTLE_COUNTER: PruningCounterRegistry<QueueKey>(
            "delayed_due_to_message_rate_throttle");
}

declare_metric! {
/// number of times a message was delayed due throttle_insert_ready_queue event
static DELAY_DUE_TO_THROTTLE_INSERT_READY_COUNTER: PruningCounterRegistry<QueueKey>(
            "delayed_due_to_throttle_insert_ready");
}

declare_metric! {
/// total number of messages across all scheduled queues
static TOTAL_DELAY_GAUGE: IntGauge("scheduled_count_total");
}

declare_metric! {
/// number of messages in the scheduled queue
static DELAY_GAUGE: PruningGaugeRegistry<QueueKey>("scheduled_count");
}

declare_metric! {
/// number of messages in the scheduled queue for a specific domain
static DOMAIN_GAUGE: PruningGaugeRegistry<DomainKey>("scheduled_by_domain");
}

declare_metric! {
/// number of messages in the scheduled queue for a specific tenant
static TENANT_GAUGE: PruningGaugeRegistry<TenantKey>("scheduled_by_tenant");
}

declare_metric! {
/// number of messages in the scheduled queue for a specific tenant and campaign combination
static TENANT_CAMPAIGN_GAUGE: PruningGaugeRegistry<TenantCampaignKey>("scheduled_by_tenant_campaign");
}

counter_bundle! {
    pub struct ScheduledCountBundle {
        pub delay_gauge: AtomicCounter,
        pub queued_by_provider: AtomicCounter,
        pub queued_by_provider_and_pool: AtomicCounter,
        pub by_domain: AtomicCounter,
    }
}

pub struct ScheduledMetrics {
    pub name: Arc<String>,
    pub scheduled: ScheduledCountBundle,
    pub by_tenant: Option<AtomicCounter>,
    pub by_tenant_campaign: Option<AtomicCounter>,
    pub delay_due_to_message_rate_throttle: OnceLock<AtomicCounter>,
    pub delay_due_to_throttle_insert_ready: OnceLock<AtomicCounter>,
    pub delay_due_to_ready_queue_full: OnceLock<AtomicCounter>,
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
                #[allow(clippy::useless_asref)]
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
