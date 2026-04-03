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
        let close_handle = this.close_handle.clone();
        lua.create_async_function(move |lua, ()| {
            let stream = stream.clone();
            let close_handle = close_handle.clone();
            async move {
                let mut guard = stream.lock().await;
                match guard.next().await {
                    Some(Ok(batch)) => {
                        let table = lua.create_table()?;
                        let options = config::serialize_options();
                        let file_name = close_handle
                            .current_file()
                            .await
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "<unknown>".to_string());
                        for (i, record) in batch.into_iter().enumerate() {
                            let json_value: serde_json::Value = serde_json::from_str(&record)
                                .map_err(|err| {
                                    any_err(format!(
                                        "Failed to parse a line from {file_name} as json: \
                                         {err}. Is the file corrupt? You may need to move \
                                         the file aside to make progress"
                                    ))
                                })?;
                            let lua_value = lua.to_value_with(&json_value, options)?;
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
        lua.create_async_function(|_lua, cfg: SerdeWrappedValue<LogTailerConfig>| async move {
            let cfg = cfg.0;
            let tailer = cfg.build().await.map_err(any_err)?;
            let close_handle = tailer.close_handle();

            Ok(LuaLogTailer {
                stream: Arc::new(Mutex::new(Box::pin(tailer))),
                close_handle,
            })
        })?,
    )?;

    Ok(())
}
