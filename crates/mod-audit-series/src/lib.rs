use config::get_or_create_sub_module;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use kumo_counter_series::{CounterSeries, CounterSeriesConfig};
use mlua::{Lua, LuaSerdeExt, UserData, UserDataMethods, Value};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

static CACHE: LazyLock<DashMap<String, Arc<Mutex<CounterSeries>>>> = LazyLock::new(DashMap::new);

struct LuaAuditSeries {
    series: Arc<Mutex<CounterSeries>>,
}

impl LuaAuditSeries {
    fn with_series<T>(
        &self,
        f: impl FnOnce(&mut CounterSeries) -> mlua::Result<T>,
    ) -> mlua::Result<T> {
        let mut guard = self
            .series
            .lock()
            .map_err(|_| mlua::Error::external("failed to acquire lock"))?;
        f(&mut guard)
    }

    fn increment(&self, to_add: u64) -> mlua::Result<()> {
        self.with_series(|series| {
            series.increment(to_add);
            Ok(())
        })
    }

    fn delta(&self, delta: i64) -> mlua::Result<()> {
        self.with_series(|series| {
            series.delta(delta);
            Ok(())
        })
    }

    fn observe(&self, value: u64) -> mlua::Result<()> {
        self.with_series(|series| {
            series.observe(value);
            Ok(())
        })
    }

    fn sum(&self) -> mlua::Result<u64> {
        self.with_series(|series| Ok(series.sum()))
    }

    fn sum_over(&self, lua: &Lua, duration: Value) -> mlua::Result<u64> {
        let duration: duration_serde::Wrap<Duration> = lua.from_value(duration)?;
        let duration = duration.into_inner();
        self.with_series(|series| Ok(series.sum_over(duration)))
    }
}

impl UserData for LuaAuditSeries {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("increment", |_lua, this, inc| this.increment(inc));
        methods.add_method("delta", |_lua, this, delta| this.delta(delta));
        methods.add_method("observe", |_lua, this, value| this.observe(value));
        methods.add_method("sum", |_lua, this, ()| this.sum());
        methods.add_method("sum_over", |lua, this, duration: Value| {
            this.sum_over(lua, duration)
        });
    }
}

fn make_config(num_buckets: u8, bucket_size_seconds: u64) -> mlua::Result<CounterSeriesConfig> {
    if num_buckets == 0 {
        return Err(mlua::Error::external("num_buckets must be >= 1"));
    }
    if bucket_size_seconds == 0 {
        return Err(mlua::Error::external("bucket_size_seconds must be >= 1"));
    }

    Ok(CounterSeriesConfig {
        num_buckets,
        bucket_size: bucket_size_seconds,
    })
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let audit_mod = get_or_create_sub_module(lua, "audit_series")?;

    audit_mod.set(
        "define",
        lua.create_function(
            |lua,
             (name, num_buckets, bucket_size, initial_value): (
                String,
                u8,
                Value,
                Option<u64>,
            )| {
                let shared = match CACHE.entry(name) {
                    Entry::Occupied(entry) => Arc::clone(entry.get()),
                    Entry::Vacant(entry) => {
                        let bucket_size: duration_serde::Wrap<Duration> =
                            lua.from_value(bucket_size)?;
                        let bucket_size_seconds = bucket_size.into_inner().as_secs();
                        let config = make_config(num_buckets, bucket_size_seconds)?;
                        let series = match initial_value {
                            Some(value) => CounterSeries::with_initial_value(config, value),
                            None => CounterSeries::with_config(config),
                        };
                        let shared = Arc::new(Mutex::new(series));
                        entry.insert(Arc::clone(&shared));
                        shared
                    }
                };

                Ok(LuaAuditSeries { series: shared })
            },
        )?,
    )?;

    Ok(())
}
