use config::{any_err, from_lua_value};
use lapin::options::BasicPublishOptions;
use lapin::publisher_confirm::{Confirmation, PublisherConfirm};
use lapin::{BasicProperties, Channel, Connection, ConnectionProperties};
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, UserDataMethods, Value};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::time::timeout;

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
    close_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[derive(Clone)]
pub struct AMQPClient {
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

        methods.add_async_method(
            "publish_with_timeout",
            |lua, this, (value, duration_millis): (Value, u64)| async move {
                let params: PublishParams = from_lua_value(lua, value)?;

                let publish = async {
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

                    wait_confirmation(lua, confirm).await
                };

                let duration = std::time::Duration::from_millis(duration_millis);
                timeout(duration, publish)
                    .await
                    .map_err(any_err)?
                    .map_err(any_err)
            },
        );

        methods.add_async_method("close", |_lua, this, _: ()| async move {
            this.holder.channel.close(200, "").await.map_err(any_err)?;
            this.holder
                .connection
                .close(200, "")
                .await
                .map_err(any_err)?;
            this.holder
                .close_tx
                .lock()
                .unwrap()
                .take()
                .map(|tx| tx.send(()));
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

async fn wait_confirmation<'lua>(
    lua: &'lua Lua,
    confirm: PublisherConfirm,
) -> mlua::Result<Value<'lua>> {
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

            wait_confirmation(lua, confirm).await
        })
    }
}

pub async fn build_client(uri: String) -> anyhow::Result<AMQPClient> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (close_tx, close_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name(format!("amqp-client-{uri}"))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()?;

            match tx.send(runtime.handle().clone()) {
                Ok(_) => {
                    runtime.block_on(close_rx)?;
                    Ok(())
                }
                Err(_) => {
                    anyhow::bail!("failed to return runtime handle");
                }
            }
        })?;
    let handle = rx.await.map_err(any_err)?;

    let options = ConnectionProperties::default()
        .with_executor(tokio_executor_trait::Tokio::default().with_handle(handle))
        .with_reactor(tokio_reactor_trait::Tokio);

    let connect_timeout = tokio::time::Duration::from_secs(20);

    let connection = timeout(connect_timeout, Connection::connect(&uri, options))
        .await
        .map_err(any_err)?
        .map_err(any_err)?;

    connection.on_error(|err| {
        tracing::error!("RabbitMQ connection broken {err:#}");
    });

    let channel = connection.create_channel().await.map_err(any_err)?;

    Ok(AMQPClient {
        holder: Arc::new(ChannelHolder {
            connection,
            channel,
            close_tx: Mutex::new(Some(close_tx)),
        }),
    })
}
