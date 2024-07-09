use config::{any_err, from_lua_value, get_or_create_sub_module};
use lapin::options::BasicPublishOptions;
use lapin::publisher_confirm::{Confirmation, PublisherConfirm};
use lapin::{BasicProperties, Channel, Connection, ConnectionProperties};
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, UserDataMethods, Value};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Deserialize, Debug)]
struct PublishParams {
    routing_key: String,
    payload: String,

    #[serde(default)]
    exchange: String,
    #[serde(default)]
    options: BasicPublishOptions,
    #[serde(default)]
    properties: BasicProperties,
}

struct ChannelHolder {
    channel: Channel,
    connection: Connection,
}

#[derive(Clone)]
struct AMQPClient {
    holder: Arc<ChannelHolder>,
}

impl LuaUserData for AMQPClient {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("publish", |lua, this, value: Value| async move {
            let params: PublishParams = from_lua_value(lua, value)?;

            let confirm = this
                .holder
                .channel
                .basic_publish(
                    &params.exchange,
                    &params.routing_key,
                    params.options,
                    params.payload.as_bytes(),
                    params.properties,
                )
                .await
                .map_err(any_err)?;

            Ok(Confirm {
                confirm: Arc::new(Mutex::new(Some(confirm))),
            })
        });

        methods.add_async_method("close", |_lua, this, _: ()| async move {
            this.holder.channel.close(200, "").await.map_err(any_err)?;
            this.holder
                .connection
                .close(200, "")
                .await
                .map_err(any_err)?;
            Ok(())
        });
    }
}

#[derive(Clone)]
struct Confirm {
    confirm: Arc<Mutex<Option<PublisherConfirm>>>,
}

#[derive(Serialize, Debug)]
enum ConfirmStatus {
    Ack,
    Nack,
    NotRequested,
}

impl ConfirmStatus {
    fn from_confirmation(confirm: &Confirmation) -> Self {
        if confirm.is_ack() {
            Self::Ack
        } else if confirm.is_nack() {
            Self::Nack
        } else {
            Self::NotRequested
        }
    }
}

#[derive(Serialize, Debug)]
struct ConfirmResult {
    status: ConfirmStatus,
    reply_code: Option<u64>,
    reply_text: Option<String>,
}

impl LuaUserData for Confirm {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("wait", |lua, this, _: ()| async move {
            let confirm = this
                .confirm
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| mlua::Error::external("confirmation already taken!?"))?;

            let confirmation = confirm.await.map_err(any_err)?;
            let status = ConfirmStatus::from_confirmation(&confirmation);
            let (reply_code, reply_text) = if let Some(msg) = confirmation.take_message() {
                (
                    Some(msg.reply_code.into()),
                    Some(msg.reply_text.as_str().to_string()),
                )
            } else {
                (None, None)
            };

            let confirmation = ConfirmResult {
                status,
                reply_code,
                reply_text,
            };

            let result = lua.to_value_with(&confirmation, config::serialize_options())?;

            Ok(result)
        })
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let amqp_mod = get_or_create_sub_module(lua, "amqp")?;

    amqp_mod.set(
        "build_client",
        lua.create_async_function(|_, uri: String| async move {
            let options = ConnectionProperties::default()
                .with_executor(tokio_executor_trait::Tokio::current())
                .with_reactor(tokio_reactor_trait::Tokio);

            let connection = Connection::connect(&uri, options).await.map_err(any_err)?;

            let channel = connection.create_channel().await.map_err(any_err)?;

            Ok(AMQPClient {
                holder: Arc::new(ChannelHolder {
                    connection,
                    channel,
                }),
            })
        })?,
    )?;

    Ok(())
}
