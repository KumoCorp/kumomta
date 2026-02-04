use kumo_prometheus::declare_metric;


macro_rules! extra {
    () => {
        " The `cache_name` label identifies which cache.  See [kumo.set_lruttl_cache_capacity](../../kumo/set_lruttl_cache_capacity.md) for a list of caches."
    }
}

declare_metric! {
/// How many times a lruttl cache lookup was initiated for a given cache.
///
#[doc = extra!()]
pub(crate) static CACHE_LOOKUP: IntCounterVec(
        "lruttl_lookup_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache evicted an item due to capacity constraints
///
#[doc = extra!()]
pub(crate) static CACHE_EVICT: IntCounterVec(
        "lruttl_evict_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache removed an item due to ttl expiration
///
#[doc = extra!()]
pub(crate) static CACHE_EXPIRE: IntCounterVec(
        "lruttl_expire_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache lookup was a hit for a given cache
///
#[doc = extra!()]
pub(crate) static CACHE_HIT: IntCounterVec(
        "lruttl_hit_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache lookup was a miss for a given cache
///
#[doc = extra!()]
pub(crate) static CACHE_MISS: IntCounterVec(
        "lruttl_miss_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache was populated via unconditional insert
///
#[doc = extra!()]
pub(crate) static CACHE_INSERT: IntCounterVec(
        "lruttl_insert_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache lookup resulted in performing the work to populate the entry
///
#[doc = extra!()]
pub(crate) static CACHE_POPULATED: IntCounterVec(
        "lruttl_populated_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache population resulted in an error
///
#[doc = extra!()]
pub(crate) static CACHE_ERROR: IntCounterVec(
        "lruttl_error_count",
        &["cache_name"]);
}

declare_metric! {
/// how many times a lruttl cache population was satisfied by a stale value
///
#[doc = extra!()]
pub(crate) static CACHE_STALE: IntCounterVec(
        "lruttl_stale_count",
        &["cache_name"]);
}

declare_metric! {
/// how many tasks are currently waiting for a cache entry to populate
///
#[doc = extra!()]
pub(crate) static CACHE_WAIT: IntGaugeVec(
        "lruttl_waiting_populate",
        &["cache_name"]);
}

declare_metric! {
/// The number of items currently contained in an lruttl cache.
///
#[doc = extra!()]
pub(crate) static CACHE_SIZE: IntGaugeVec(
        "lruttl_cache_size",
        &["cache_name"]);
}
