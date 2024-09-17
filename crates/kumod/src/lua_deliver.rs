use crate::delivery_metrics::MetricsWrappedConnection;
use crate::logging::disposition::{log_disposition, LogDisposition};
use crate::ready_queue::{Dispatcher, QueueDispatcher};
use crate::smtp_server::RejectError;
use crate::spool::SpoolManager;
use async_trait::async_trait;
use config::{CallbackSignature, LuaConfig};
use kumo_log_types::{RecordType, ResolvedAddress};
use kumo_server_runtime::spawn_local;
use message::message::QueueNameComponents;
use message::Message;
use mlua::{RegistryKey, Value};
use rfc5321::Response;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LuaDeliveryProtocol {
    /// The name of an event to fire that will construct
    /// the delivery implementation
    pub constructor: String,

    #[serde(default = "LuaDeliveryProtocol::default_batch_size")]
    batch_size: usize,
}

impl LuaDeliveryProtocol {
    fn default_batch_size() -> usize {
        1
    }
}

#[derive(Debug)]
enum ConnectionState {
    NotYet,
    Connected(MetricsWrappedConnection<RegistryKey>),
    Disconnected,
}

impl ConnectionState {
    fn take(&mut self) -> Option<MetricsWrappedConnection<RegistryKey>> {
        match std::mem::replace(self, Self::Disconnected) {
            Self::NotYet | Self::Disconnected => None,
            Self::Connected(c) => Some(c),
        }
    }
}

#[derive(Debug)]
pub struct LuaQueueDispatcher {
    lua_config: LuaConfig,
    proto_config: LuaDeliveryProtocol,
    connection: ConnectionState,
    peer_address: ResolvedAddress,
}

impl LuaQueueDispatcher {
    pub fn new(lua_config: LuaConfig, proto_config: LuaDeliveryProtocol) -> Self {
        let peer_address = ResolvedAddress {
            name: format!("Lua via {}", proto_config.constructor),
            addr: Ipv4Addr::UNSPECIFIED.into(),
        };

        Self {
            lua_config,
            proto_config,
            connection: ConnectionState::NotYet,
            peer_address,
        }
    }
}

#[async_trait(?Send)]
impl QueueDispatcher for LuaQueueDispatcher {
    async fn close_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<bool> {
        tracing::debug!("close_connection called");
        if let Some(connection) = self.connection.take() {
            tracing::debug!("will try close method");
            let result: anyhow::Result<()> = self
                .lua_config
                .with_registry_value(&connection, move |connection| {
                    Ok(async move {
                        match &connection {
                            Value::Table(tbl) => match tbl.get("close")? {
                                mlua::Value::Function(close_method) => {
                                    Ok(close_method.call_async(connection).await?)
                                }
                                _ => Ok(()),
                            },
                            _ => anyhow::bail!("invalid connection object"),
                        }
                    })
                })
                .await;

            if let Err(err) = result {
                tracing::error!(
                    "Error while closing connection for {}: {err:#}",
                    dispatcher.name
                );
            }

            self.lua_config.remove_registry_value(connection.take())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn max_batch_size(&self) -> usize {
        self.proto_config.batch_size
    }

    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        match &self.connection {
            ConnectionState::Connected(_) => return Ok(()),
            ConnectionState::Disconnected => {
                anyhow::bail!("only one connection attempt per session");
            }
            ConnectionState::NotYet => {}
        };
        let connection_wrapper = dispatcher.metrics.wrap_connection(());
        // Normally, a QueueDispatcher would use dispatcher.path_config rather
        // than resolving through queue_name_for_config_change_purposes_only.
        // In this case, since LuaQueueDispatcher doesn't have multiple egress
        // sources, it is acceptable to use queue_name_for_config_change_purposes_only.
        let components =
            QueueNameComponents::parse(&dispatcher.queue_name_for_config_change_purposes_only);
        let sig = CallbackSignature::<(&str, Option<&str>, Option<&str>), Value>::new(
            self.proto_config.constructor.to_string(),
        );

        let connection = self
            .lua_config
            .async_call_ctor(
                &sig,
                (components.domain, components.tenant, components.campaign),
            )
            .await?;

        self.connection = ConnectionState::Connected(connection_wrapper.map_connection(connection));
        dispatcher.delivered_this_connection = 0;
        Ok(())
    }

    async fn have_more_connection_candidates(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        false
    }

    async fn deliver_message(
        &mut self,
        mut msgs: Vec<Message>,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()> {
        let connection = match &self.connection {
            ConnectionState::Connected(c) => c,
            _ => {
                anyhow::bail!("connection is not set in LuaQueueDispatcher::deliver_message!?");
            }
        };

        let batch_size = self.proto_config.batch_size;

        let result: anyhow::Result<String> = self
            .lua_config
            .with_registry_value(connection, move |connection| {
                Ok(async move {
                    match &connection {
                        Value::Table(tbl) => {
                            if batch_size == 1 {
                                anyhow::ensure!(
                                    msgs.len() == 1,
                                    "lua_dispatcher was configured with a batch size of 1, but multiple messages were popped"
                                );
                                let msg = msgs.pop().expect("just verified that there is one");
                                let send_method: mlua::Function = tbl.get("send").context("sender:send method not found")?;

                                Ok(send_method.call_async((connection, msg)).await.context("sender:send method failed")?)
                            } else {
                                let send_method: mlua::Function = tbl.get("send_batch").context("sender:send_batch method not found")?;

                                Ok(send_method.call_async((connection, msgs)).await.context("sender:send_batch method failed")?)
                            }
                        }
                        _ => anyhow::bail!("invalid connection object"),
                    }
                })
            })
            .await;

        match result {
            Err(err) => {
                if let Some(rejection) = RejectError::from_anyhow(&err) {
                    let response = Response {
                        code: rejection.code,
                        enhanced_code: None,
                        content: rejection.message.to_string(),
                        command: None,
                    };

                    if rejection.code >= 400 && rejection.code < 500 {
                        // Explicit Transient failure
                        tracing::debug!(
                            "failed to send message to {}: {response:?}",
                            dispatcher.name,
                        );
                        for msg in dispatcher.msgs.drain(..) {
                            log_disposition(LogDisposition {
                                kind: RecordType::TransientFailure,
                                msg: msg.clone(),
                                site: &dispatcher.name,
                                peer_address: Some(&self.peer_address),
                                response: response.clone(),
                                egress_pool: Some(&dispatcher.egress_pool),
                                egress_source: Some(&dispatcher.egress_source.name),
                                relay_disposition: None,
                                delivery_protocol: Some("Lua"),
                                tls_info: None,
                                source_address: None,
                                provider: dispatcher.path_config.borrow().provider_name.as_deref(),
                            })
                            .await;
                            spawn_local(
                                "requeue message".to_string(),
                                Dispatcher::requeue_message(msg, true, None),
                            )?;
                            dispatcher.metrics.inc_transfail();
                        }

                        if rejection.code == 421 {
                            // Explicit signal that we want to close
                            if let Err(close_err) = self.close_connection(dispatcher).await {
                                tracing::debug!("error while closing {close_err:#}");
                            }
                        }
                    } else {
                        tracing::debug!(
                            "failed to send message to {}: {response:?}",
                            dispatcher.name,
                        );
                        for msg in dispatcher.msgs.drain(..) {
                            dispatcher.metrics.inc_fail();
                            log_disposition(LogDisposition {
                                kind: RecordType::Bounce,
                                msg: msg.clone(),
                                site: &dispatcher.name,
                                peer_address: Some(&self.peer_address),
                                response: response.clone(),
                                egress_pool: Some(&dispatcher.egress_pool),
                                egress_source: Some(&dispatcher.egress_source.name),
                                relay_disposition: None,
                                delivery_protocol: Some("Lua"),
                                tls_info: None,
                                source_address: None,
                                provider: dispatcher.path_config.borrow().provider_name.as_deref(),
                            })
                            .await;
                            SpoolManager::remove_from_spool(*msg.id()).await?;
                        }
                    }
                } else {
                    // unspecified failure
                    tracing::debug!("failed to send message to {}: {err:#}", dispatcher.name);
                    if let Err(close_err) = self.close_connection(dispatcher).await {
                        tracing::debug!("error while closing {close_err:#}");
                    }
                    return Err(err);
                }
            }
            Ok(response) => {
                let response = Response {
                    code: 200,
                    enhanced_code: None,
                    content: response,
                    command: None,
                };

                tracing::debug!("Delivered OK! {response:?}");
                for msg in dispatcher.msgs.drain(..) {
                    log_disposition(LogDisposition {
                        kind: RecordType::Delivery,
                        msg: msg.clone(),
                        site: &dispatcher.name,
                        peer_address: Some(&self.peer_address),
                        response: response.clone(),
                        egress_pool: Some(&dispatcher.egress_pool),
                        egress_source: Some(&dispatcher.egress_source.name),
                        relay_disposition: None,
                        delivery_protocol: Some("Lua"),
                        tls_info: None,
                        source_address: None,
                        provider: dispatcher.path_config.borrow().provider_name.as_deref(),
                    })
                    .await;
                    SpoolManager::remove_from_spool(*msg.id()).await?;
                    dispatcher.metrics.inc_delivered();
                }
            }
        }

        Ok(())
    }
}
