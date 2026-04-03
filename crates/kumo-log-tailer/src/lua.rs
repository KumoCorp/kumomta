use crate::{CloseHandle, LogTailerConfig};
use config::{any_err, get_or_create_sub_module, SerdeWrappedValue};
use futures::StreamExt;
use mlua::{Lua, LuaSerdeExt, UserData, UserDataMethods, UserDataRef, UserDataRefMut};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Wrapper around [`LogTailer`] exposed to Lua.
///
/// The stream is driven on each call to the iterator returned by
/// `:batches()`.  A `tokio::sync::Mutex` provides interior mutability
/// for the `Fn` closure required by mlua; it is never contended because
/// there is only a single consumer.
struct LuaLogTailer {
    stream: Arc<Mutex<Pin<Box<crate::LogTailer>>>>,
    close_handle: CloseHandle,
}

impl LuaLogTailer {
    async fn close(_lua: Lua, this: UserDataRefMut<Self>, _: ()) -> mlua::Result<()> {
        this.close_handle.close().await.map_err(any_err)
    }

    /// Returns an async iterator function suitable for use with
    /// Lua's generic `for` loop.  Each call to the returned function
    /// polls the underlying stream for the next batch.
    async fn batches(lua: Lua, this: UserDataRef<Self>, _: ()) -> mlua::Result<mlua::Function> {
        let stream = this.stream.clone();
        lua.create_async_function(move |lua, ()| {
            let stream = stream.clone();
            async move {
                let mut guard = stream.lock().await;
                match guard.next().await {
                    Some(Ok(batch)) => {
                        let table = lua.create_table()?;
                        let options = config::serialize_options();
                        for (i, value) in batch.records().iter().enumerate() {
                            let lua_value = lua.to_value_with(value, options)?;
                            table.raw_set(i + 1, lua_value)?;
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

impl UserData for LuaLogTailer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("batches", Self::batches);
        methods.add_async_method_mut("close", Self::close);
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let tailer_mod = get_or_create_sub_module(lua, "tailer")?;

    tailer_mod.set(
        "new",
        lua.create_async_function(
            |lua,
             (cfg, lua_filter): (SerdeWrappedValue<LogTailerConfig>, Option<mlua::Function>)| async move {
                let cfg = cfg.0;

                let filter = match lua_filter {
                    Some(func) => {
                        let lua = lua.clone();
                        let filter_fn =
                            move |value: &serde_json::Value| -> anyhow::Result<bool> {
                                let options = config::serialize_options();
                                let lua_value = lua.to_value_with(value, options)?;
                                let result: bool = func.call(lua_value.clone())?;
                                Ok(result)
                            };
                        Some(filter_fn)
                    }
                    None => None,
                };

                let tailer = cfg.build_with_filter(filter).await.map_err(any_err)?;
                let close_handle = tailer.close_handle();

                Ok(LuaLogTailer {
                    stream: Arc::new(Mutex::new(Box::pin(tailer))),
                    close_handle,
                })
            },
        )?,
    )?;

    Ok(())
}
