use crate::{
    CloseHandle, ConsumerConfig, LogBatch, LogTailerConfig, LogWriterConfig,
    MultiConsumerTailerConfig,
};
use config::{any_err, from_lua_value, get_or_create_sub_module, SerdeWrappedValue};
use futures::StreamExt;
use mlua::{Lua, LuaSerdeExt, MetaMethod, UserData, UserDataMethods, UserDataRef, UserDataRefMut};
use serde::Deserialize;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// LuaLogBatch
// ---------------------------------------------------------------------------

/// Lua wrapper around [`LogBatch`].
///
/// Exposes `:records()` to iterate over the parsed JSON records,
/// `:iter_records()` for lazy iteration, `:consumer_name()` to
/// identify which consumer the batch belongs to, and `:commit()`
/// to advance the checkpoint.
struct LuaLogBatch {
    inner: Option<LogBatch>,
}

impl LuaLogBatch {
    /// Return the consumer name for this batch.
    /// Returns an empty string if the batch has already been committed.
    fn consumer_name(_lua: &Lua, this: &Self, _: ()) -> mlua::Result<String> {
        Ok(this
            .inner
            .as_ref()
            .map(|b| b.consumer_name().to_string())
            .unwrap_or_default())
    }

    /// Return the records as a lua table (sequence of parsed JSON values).
    /// Returns an empty table if the batch has already been committed.
    fn records(lua: &Lua, this: &Self, _: ()) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;
        if let Some(batch) = &this.inner {
            let options = config::serialize_options();
            for (i, value) in batch.records().iter().enumerate() {
                let lua_value = lua.to_value_with(value, options)?;
                table.raw_set(i + 1, lua_value)?;
            }
        }
        Ok(mlua::Value::Table(table))
    }

    /// Advance the checkpoint to the end of this batch.
    /// No-op if the batch has already been committed.
    fn commit(_lua: &Lua, this: &mut Self, _: ()) -> mlua::Result<()> {
        if let Some(mut batch) = this.inner.take() {
            batch.commit().map_err(any_err)?;
        }
        Ok(())
    }

    /// Return an iterator function that yields one record at a time,
    /// converting each JSON value to a lua value lazily on demand.
    /// Returns an iterator that immediately yields nil if the batch
    /// has already been committed.
    fn iter_records(lua: &Lua, this: &Self, _: ()) -> mlua::Result<mlua::Function> {
        let records = match &this.inner {
            Some(batch) => batch.records().to_vec(),
            None => Vec::new(),
        };
        let records = std::sync::Arc::new(records);
        let idx = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        lua.create_function(move |lua, ()| {
            let i = idx.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Some(value) = records.get(i) {
                let options = config::serialize_options();
                let lua_value = lua.to_value_with(value, options)?;
                Ok(lua_value)
            } else {
                Ok(mlua::Value::Nil)
            }
        })
    }
}

impl UserData for LuaLogBatch {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("consumer_name", Self::consumer_name);
        methods.add_method("records", Self::records);
        methods.add_method("iter_records", Self::iter_records);
        methods.add_method_mut("commit", Self::commit);
    }
}

// ---------------------------------------------------------------------------
// LuaLogTailer (single-consumer)
// ---------------------------------------------------------------------------

struct LuaLogTailer {
    stream: Arc<Mutex<Pin<Box<crate::LogTailer>>>>,
    close_handle: CloseHandle,
}

impl LuaLogTailer {
    async fn close(_lua: Lua, this: UserDataRefMut<Self>, _: ()) -> mlua::Result<()> {
        this.close_handle.close();
        Ok(())
    }

    async fn batches(lua: Lua, this: UserDataRef<Self>, _: ()) -> mlua::Result<mlua::Function> {
        let stream = this.stream.clone();
        lua.create_async_function(move |lua, ()| {
            let stream = stream.clone();
            async move {
                let mut guard = stream.lock().await;
                match guard.next().await {
                    Some(Ok(batch)) => Ok(mlua::Value::UserData(
                        lua.create_userdata(LuaLogBatch { inner: Some(batch) })?,
                    )),
                    Some(Err(e)) => Err(any_err(e)),
                    None => Ok(mlua::Value::Nil),
                }
            }
        })
    }
}

impl UserData for LuaLogTailer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("batches", Self::batches);
        methods.add_async_method_mut("close", Self::close);
        methods.add_async_meta_method_mut(MetaMethod::Close, Self::close);
    }
}

// ---------------------------------------------------------------------------
// LuaMultiConsumerTailer
// ---------------------------------------------------------------------------

struct LuaMultiConsumerTailer {
    stream: Arc<Mutex<Pin<Box<crate::MultiConsumerTailer>>>>,
    close_handle: CloseHandle,
}

impl LuaMultiConsumerTailer {
    async fn close(_lua: Lua, this: UserDataRefMut<Self>, _: ()) -> mlua::Result<()> {
        this.close_handle.close();
        Ok(())
    }

    /// Returns an async iterator function that yields a lua table
    /// (sequence) of [`LuaLogBatch`] userdata values on each call,
    /// or nil when the stream is exhausted.
    async fn batches(lua: Lua, this: UserDataRef<Self>, _: ()) -> mlua::Result<mlua::Function> {
        let stream = this.stream.clone();
        lua.create_async_function(move |lua, ()| {
            let stream = stream.clone();
            async move {
                let mut guard = stream.lock().await;
                match guard.next().await {
                    Some(Ok(batch_vec)) => {
                        let table = lua.create_table()?;
                        for (i, batch) in batch_vec.into_iter().enumerate() {
                            let ud = lua.create_userdata(LuaLogBatch { inner: Some(batch) })?;
                            table.raw_set(i + 1, ud)?;
                        }
                        Ok(mlua::Value::Table(table))
                    }
                    Some(Err(e)) => Err(any_err(e)),
                    None => Ok(mlua::Value::Nil),
                }
            }
        })
    }
}

impl UserData for LuaMultiConsumerTailer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("batches", Self::batches);
        methods.add_async_method_mut("close", Self::close);
        methods.add_async_meta_method_mut(MetaMethod::Close, Self::close);
    }
}

// ---------------------------------------------------------------------------
// Lua-facing config for multi-consumer (deserialized from a lua table)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LuaConsumerConfig {
    name: String,
    #[serde(default = "default_max_batch_size")]
    max_batch_size: usize,
    #[serde(default = "default_max_batch_latency", with = "duration_serde")]
    max_batch_latency: Duration,
    #[serde(default)]
    checkpoint_name: Option<String>,
}

fn default_max_batch_size() -> usize {
    100
}

fn default_max_batch_latency() -> Duration {
    Duration::from_secs(1)
}

#[derive(Deserialize)]
struct LuaMultiConsumerTailerConfig {
    directory: String,
    #[serde(default = "default_pattern")]
    pattern: String,
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    poll_watcher: Option<Duration>,
    #[serde(default)]
    tail: bool,
    // consumers is extracted manually from the lua table so that
    // we can handle the filter function field which serde can't
    // deserialize.
}

fn default_pattern() -> String {
    "*".to_string()
}

/// Helper to build a Rust filter closure from a lua function.
fn make_rust_filter(
    lua: &Lua,
    func: mlua::Function,
) -> Box<dyn Fn(&serde_json::Value) -> anyhow::Result<bool> + Send> {
    let lua = lua.clone();
    Box::new(move |value: &serde_json::Value| -> anyhow::Result<bool> {
        let options = config::serialize_options();
        let lua_value = lua.to_value_with(value, options)?;
        let result: bool = func.call(lua_value.clone())?;
        Ok(result)
    })
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let tailer_mod = get_or_create_sub_module(lua, "tailer")?;

    // kumo.tailer.new — single-consumer tailer
    tailer_mod.set(
        "new",
        lua.create_async_function(
            |lua,
             (cfg, lua_filter): (
                SerdeWrappedValue<LogTailerConfig>,
                Option<mlua::Function>,
            )| async move {
                let cfg = cfg.0;
                let filter = lua_filter.map(|f| make_rust_filter(&lua, f));
                let tailer = cfg.build_with_filter(filter).await.map_err(any_err)?;
                let close_handle = tailer.close_handle();

                Ok(LuaLogTailer {
                    stream: Arc::new(Mutex::new(Box::pin(tailer))),
                    close_handle,
                })
            },
        )?,
    )?;

    // kumo.tailer.new_multi — multi-consumer tailer
    //
    // Usage:
    //   local tailer <close> = kumo.tailer.new_multi {
    //     directory = '/var/log/kumomta',
    //     consumers = {
    //       { name = 'deliveries', checkpoint_name = 'cp-del',
    //         filter = function(record) return record.type == 'Delivery' end },
    //       { name = 'bounces', checkpoint_name = 'cp-bounce',
    //         filter = function(record) return record.type == 'Bounce' end },
    //     },
    //   }
    //
    //   for batches in tailer:batches() do
    //     for _, batch in ipairs(batches) do
    //       print(batch:consumer_name())
    //       for record in batch:iter_records() do ... end
    //       batch:commit()
    //     end
    //   end
    tailer_mod.set(
        "new_multi",
        lua.create_async_function(|lua, cfg_value: mlua::Value| async move {
            let cfg_table = match &cfg_value {
                mlua::Value::Table(t) => t,
                _ => return Err(any_err("expected a table")),
            };

            // Remove consumers before serde deserialization because
            // the consumer entries may contain lua function values
            // that serde cannot handle.
            let consumers_table: mlua::Table = cfg_table.get("consumers")?;
            cfg_table.set("consumers", mlua::Value::Nil)?;

            // Deserialize the top-level non-function fields via serde
            let cfg: LuaMultiConsumerTailerConfig = from_lua_value(&lua, cfg_value)?;

            let num_consumers = consumers_table.len()? as usize;

            let mut consumers = Vec::with_capacity(num_consumers);
            for idx in 0..num_consumers {
                let entry: mlua::Table = consumers_table.get(idx + 1)?;
                // Extract the filter function before serde sees the table
                let filter_func: Option<mlua::Function> =
                    entry.get::<mlua::Function>("filter").ok();
                entry.set("filter", mlua::Value::Nil)?;
                let lc: LuaConsumerConfig = from_lua_value(&lua, mlua::Value::Table(entry))?;

                let mut consumer = ConsumerConfig::new(&lc.name)
                    .max_batch_size(lc.max_batch_size)
                    .max_batch_latency(lc.max_batch_latency);
                if let Some(cp) = lc.checkpoint_name {
                    consumer = consumer.checkpoint_name(cp);
                }
                if let Some(func) = filter_func {
                    consumer = consumer.filter(make_rust_filter(&lua, func));
                }
                consumers.push(consumer);
            }

            let multi_cfg = MultiConsumerTailerConfig::new(cfg.directory.into(), consumers)
                .pattern(cfg.pattern)
                .tail(cfg.tail);
            let multi_cfg = match cfg.poll_watcher {
                Some(interval) => multi_cfg.poll_watcher(interval),
                None => multi_cfg,
            };

            let tailer = multi_cfg.build().await.map_err(any_err)?;
            let close_handle = tailer.close_handle();

            Ok(LuaMultiConsumerTailer {
                stream: Arc::new(Mutex::new(Box::pin(tailer))),
                close_handle,
            })
        })?,
    )?;

    // kumo.tailer.new_writer — log file writer
    tailer_mod.set(
        "new_writer",
        lua.create_function(|lua, cfg: SerdeWrappedValue<LogWriterConfig>| {
            let _ = lua;
            Ok(LuaLogWriter {
                inner: Some(cfg.0.build()),
            })
        })?,
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// LuaLogWriter
// ---------------------------------------------------------------------------

/// Lua wrapper around [`crate::writer::LogWriter`].
struct LuaLogWriter {
    inner: Option<crate::writer::LogWriter>,
}

impl LuaLogWriter {
    fn write_line(_lua: &Lua, this: &mut Self, line: String) -> mlua::Result<()> {
        let writer = this
            .inner
            .as_mut()
            .ok_or_else(|| mlua::Error::external("writer has been closed"))?;
        writer.write_line(&line).map_err(any_err)
    }

    fn write_record(lua: &Lua, this: &mut Self, value: mlua::Value) -> mlua::Result<()> {
        let writer = this
            .inner
            .as_mut()
            .ok_or_else(|| mlua::Error::external("writer has been closed"))?;
        // Convert lua value to serde_json::Value, then serialize to JSON string
        let json_value: serde_json::Value = lua.from_value(value)?;
        writer.write_value(&json_value).map_err(any_err)
    }

    fn close(_lua: &Lua, this: &mut Self, _: ()) -> mlua::Result<()> {
        if let Some(mut writer) = this.inner.take() {
            writer.close().map_err(any_err)?;
        }
        Ok(())
    }
}

impl UserData for LuaLogWriter {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("write_line", Self::write_line);
        methods.add_method_mut("write_record", Self::write_record);
        methods.add_method_mut("close", Self::close);
        methods.add_meta_method_mut(MetaMethod::Close, Self::close);
    }
}
