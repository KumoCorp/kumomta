use prometheus::{IntCounterVec, IntGaugeVec};
use std::sync::LazyLock;

pub(crate) static CACHE_LOOKUP: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_lookup_count",
        "how many times a lruttl cache lookup was initiated for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_EVICT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_evict_count",
        "how many times a lruttl cache evicted an item due to capacity constraints",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_EXPIRE: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_expire_count",
        "how many times a lruttl cache removed an item due to ttl expiration",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_HIT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_hit_count",
        "how many times a lruttl cache lookup was a hit for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_MISS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_miss_count",
        "how many times a lruttl cache lookup was a miss for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_INSERT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_insert_count",
        "how many times a lruttl cache was populated via unconditional insert",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_POPULATED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_populated_count",
        "how many times a lruttl cache lookup resulted in performing the work to populate the entry",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_ERROR: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_error_count",
        "how many times a lruttl cache population resulted in an error",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_STALE: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_stale_count",
        "how many times a lruttl cache population was satisfied by a stale value",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_WAIT: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "lruttl_waiting_populate",
        "how many tasks are currently waiting for a cache entry to populate",
        &["cache_name"]
    )
    .unwrap()
});
pub(crate) static CACHE_SIZE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "lruttl_cache_size",
        "number of items contained in an lruttl cache",
        &["cache_name"]
    )
    .unwrap()
});
