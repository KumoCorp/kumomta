use config::{SerdeWrappedValue, get_or_create_sub_module};
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use kumo_counter_series::{CounterSeries, CounterSeriesConfig};
use mlua::{Lua, UserData, UserDataMethods};
use parking_lot::Mutex;
use serde::Deserialize;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

type LuaDuration = SerdeWrappedValue<duration_serde::Wrap<Duration>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefineParams {
    name: String,
    num_buckets: u8,
    #[serde(with = "duration_serde")]
    bucket_size: Duration,
    #[serde(default)]
    initial_value: Option<u64>,
}

struct CachedSeries {
    num_buckets: u8,
    bucket_size_seconds: u64,
    series: Arc<Mutex<CounterSeries>>,
}

static CACHE: LazyLock<DashMap<String, CachedSeries>> = LazyLock::new(DashMap::new);

struct LuaCounterSeries {
    series: Arc<Mutex<CounterSeries>>,
}

impl UserData for LuaCounterSeries {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("increment", |_lua, this, inc: u64| {
            this.series.lock().increment(inc);
            Ok(())
        });
        methods.add_method("delta", |_lua, this, delta: i64| {
            this.series.lock().delta(delta);
            Ok(())
        });
        methods.add_method("observe", |_lua, this, value: u64| {
            this.series.lock().observe(value);
            Ok(())
        });
        methods.add_method("sum", |_lua, this, ()| Ok(this.series.lock().sum()));
        methods.add_method("sum_over", |_lua, this, duration: LuaDuration| {
            Ok(this.series.lock().sum_over((*duration).into_inner()))
        });
    }
}

/// Rounds a `Duration` up to the nearest whole second. A duration with any
/// non-zero sub-second component is rounded up to the next full second.
fn round_up_to_seconds(d: Duration) -> u64 {
    let secs = d.as_secs();
    if d.subsec_nanos() > 0 { secs + 1 } else { secs }
}

fn make_config(num_buckets: u8, bucket_size_seconds: u64) -> mlua::Result<CounterSeriesConfig> {
    if num_buckets == 0 {
        return Err(mlua::Error::external("num_buckets must be >= 1"));
    }
    if bucket_size_seconds == 0 {
        return Err(mlua::Error::external("bucket_size must be >= 1 second"));
    }

    Ok(CounterSeriesConfig {
        num_buckets,
        bucket_size: bucket_size_seconds,
    })
}

fn build_cached(
    num_buckets: u8,
    bucket_size_seconds: u64,
    initial_value: Option<u64>,
) -> mlua::Result<CachedSeries> {
    let config = make_config(num_buckets, bucket_size_seconds)?;
    let series = match initial_value {
        Some(value) => CounterSeries::with_initial_value(config, value),
        None => CounterSeries::with_config(config),
    };
    Ok(CachedSeries {
        num_buckets,
        bucket_size_seconds,
        series: Arc::new(Mutex::new(series)),
    })
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let module = get_or_create_sub_module(lua, "counter_series")?;

    module.set(
        "define",
        lua.create_function(|_lua, params: SerdeWrappedValue<DefineParams>| {
            let DefineParams {
                name,
                num_buckets,
                bucket_size,
                initial_value,
            } = params.0;
            let bucket_size_seconds = round_up_to_seconds(bucket_size);

            let series = match CACHE.entry(name) {
                Entry::Occupied(mut entry) => {
                    let cached = entry.get();
                    if cached.num_buckets == num_buckets
                        && cached.bucket_size_seconds == bucket_size_seconds
                    {
                        // Same shape: preserve existing values, ignore initial_value.
                        Arc::clone(&cached.series)
                    } else {
                        // Shape changed: replace the cached series with a fresh one.
                        let fresh = build_cached(num_buckets, bucket_size_seconds, initial_value)?;
                        let series = Arc::clone(&fresh.series);
                        entry.insert(fresh);
                        series
                    }
                }
                Entry::Vacant(entry) => {
                    let fresh = build_cached(num_buckets, bucket_size_seconds, initial_value)?;
                    let series = Arc::clone(&fresh.series);
                    entry.insert(fresh);
                    series
                }
            };

            Ok(LuaCounterSeries { series })
        })?,
    )?;

    Ok(())
}
