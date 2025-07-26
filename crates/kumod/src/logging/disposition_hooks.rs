use crate::logging::LogRecordParams;
use config::{load_config, CallbackSignature};
use kumo_log_types::{JsonLogRecord, RecordType};
use message::Message;
use mlua::{IntoLua, LuaSerdeExt};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Clone)]
pub struct RecordWrapper(JsonLogRecord);

impl IntoLua for RecordWrapper {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        lua.to_value(&self.0)
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct DispHookParams {
    /// The unique name to identify this instance of the log hook
    pub name: String,

    #[serde(default)]
    pub per_record: HashMap<RecordType, LogRecordParams>,
}

impl DispHookParams {
    pub async fn do_record(
        sig: &CallbackSignature<(Message, RecordWrapper), ()>,
        msg: Message,
        record: JsonLogRecord,
    ) -> anyhow::Result<()> {
        tracing::trace!("do_record {record:?}");

        if record.reception_protocol.as_deref() == Some("LogRecord") {
            return Ok(());
        }

        let mut lua_config = load_config().await?;
        lua_config
            .async_call_callback(&sig, (msg.clone(), RecordWrapper(record)))
            .await?;
        lua_config.put();
        anyhow::Result::<()>::Ok(())
    }
}
