use crate::delivery_metrics::MetricsWrappedConnection;
use crate::logging::{log_disposition, LogDisposition};
use crate::ready_queue::{Dispatcher, QueueDispatcher};
use crate::smtp_server::RejectError;
use crate::spool::SpoolManager;
use async_trait::async_trait;
use config::{CallbackSignature, LuaConfig};
use kumo_log_types::{RecordType, ResolvedAddress};
use kumo_server_runtime::{rt_spawn, spawn};
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
}

#[derive(Debug)]
pub struct LuaQueueDispatcher {
    lua_config: LuaConfig,
    proto_config: LuaDeliveryProtocol,
    connection: Option<MetricsWrappedConnection<RegistryKey>>,
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
            connection: None,
            peer_address,
        }
    }
}

#[async_trait(?Send)]
impl QueueDispatcher for LuaQueueDispatcher {
    async fn close_connection(&mut self, _dispatcher: &mut Dispatcher) -> anyhow::Result<bool> {
        if let Some(connection) = self.connection.take() {
            self.lua_config.remove_registry_value(connection.take())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        let connection_wrapper = dispatcher.metrics.wrap_connection(());
        let components = QueueNameComponents::parse(&dispatcher.queue_name);
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

        self.connection
            .replace(connection_wrapper.map_connection(connection));
        Ok(())
    }

    async fn have_more_connection_candidates(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        false
    }

    async fn deliver_message(
        &mut self,
        msg: Message,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()> {
        let connection = self.connection.as_ref().ok_or_else(|| {
            anyhow::anyhow!("connection is not set in LuaQueueDispatcher::deliver_message!?")
        })?;

        let result: anyhow::Result<String> = self
            .lua_config
            .with_registry_value(connection, move |connection| {
                Ok(async move {
                    match &connection {
                        Value::Table(tbl) => {
                            let send_method: mlua::Function = tbl.get("send")?;

                            Ok(send_method.call_async((connection, msg)).await?)
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
                        if let Some(msg) = dispatcher.msg.take() {
                            log_disposition(LogDisposition {
                                kind: RecordType::TransientFailure,
                                msg: msg.clone(),
                                site: &dispatcher.name,
                                peer_address: Some(&self.peer_address),
                                response,
                                egress_pool: Some(&dispatcher.egress_pool),
                                egress_source: Some(&dispatcher.egress_source.name),
                                relay_disposition: None,
                                delivery_protocol: Some("Lua"),
                                tls_info: None,
                            })
                            .await;
                            rt_spawn("requeue message".to_string(), move || {
                                Ok(async move { Dispatcher::requeue_message(msg, true, None).await })
                            })
                            .await?;
                        }
                        dispatcher.metrics.inc_transfail();
                    } else {
                        dispatcher.metrics.inc_fail();
                        tracing::debug!(
                            "failed to send message to {}: {response:?}",
                            dispatcher.name,
                        );
                        if let Some(msg) = dispatcher.msg.take() {
                            log_disposition(LogDisposition {
                                kind: RecordType::Bounce,
                                msg: msg.clone(),
                                site: &dispatcher.name,
                                peer_address: Some(&self.peer_address),
                                response,
                                egress_pool: Some(&dispatcher.egress_pool),
                                egress_source: Some(&dispatcher.egress_source.name),
                                relay_disposition: None,
                                delivery_protocol: Some("Lua"),
                                tls_info: None,
                            })
                            .await;
                            spawn("remove from spool", async move {
                                SpoolManager::remove_from_spool(*msg.id()).await
                            })?;
                        }
                    }
                } else {
                    // unspecified failure
                    tracing::debug!("failed to send message to {}: {err:#}", dispatcher.name);
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
                if let Some(msg) = dispatcher.msg.take() {
                    log_disposition(LogDisposition {
                        kind: RecordType::Delivery,
                        msg: msg.clone(),
                        site: &dispatcher.name,
                        peer_address: Some(&self.peer_address),
                        response,
                        egress_pool: Some(&dispatcher.egress_pool),
                        egress_source: Some(&dispatcher.egress_source.name),
                        relay_disposition: None,
                        delivery_protocol: Some("Lua"),
                        tls_info: None,
                    })
                    .await;
                    spawn("remove from spool", async move {
                        SpoolManager::remove_from_spool(*msg.id()).await
                    })?;
                }
                dispatcher.metrics.inc_delivered();
            }
        }

        Ok(())
    }
}
