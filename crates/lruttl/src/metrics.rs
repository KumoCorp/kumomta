use kumo_prometheus::declare_metric;

declare_metric! {
/// how many times a lruttl cache lookup was initiated for a given cache
pub(crate) static CACHE_LOOKUP: IntCounterVec(
        "lruttl_lookup_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache evicted an item due to capacity constraints
pub(crate) static CACHE_EVICT: IntCounterVec(
        "lruttl_evict_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache removed an item due to ttl expiration
pub(crate) static CACHE_EXPIRE: IntCounterVec(
        "lruttl_expire_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache lookup was a hit for a given cache
pub(crate) static CACHE_HIT: IntCounterVec(
        "lruttl_hit_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache lookup was a miss for a given cache
pub(crate) static CACHE_MISS: IntCounterVec(
        "lruttl_miss_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache was populated via unconditional insert
pub(crate) static CACHE_INSERT: IntCounterVec(
        "lruttl_insert_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache lookup resulted in performing the work to populate the entry
pub(crate) static CACHE_POPULATED: IntCounterVec(
        "lruttl_populated_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache population resulted in an error
pub(crate) static CACHE_ERROR: IntCounterVec(
        "lruttl_error_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache population was satisfied by a stale value
pub(crate) static CACHE_STALE: IntCounterVec(
        "lruttl_stale_count",
        &["cache_name"]);
}

declare_metric! {
        /// how many tasks are currently waiting for a cache entry to populate
pub(crate) static CACHE_WAIT: IntGaugeVec(
        "lruttl_waiting_populate",
        &["cache_name"]);
}

declare_metric! {
/// number of items contained in an lruttl cache
pub(crate) static CACHE_SIZE: IntGaugeVec(
        "lruttl_cache_size",
        &["cache_name"]);
}
