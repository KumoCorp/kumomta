use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::InsertContext;
use crate::xfer::{QueueManager, SavedQueueInfo};
use config::{any_err, get_or_create_sub_module, SerdeWrappedValue};
use kumo_api_types::xfer::XferProtocol;
use message::Message;
use mlua::{Lua, UserDataRef};
use mod_time::TimeDelta;
use reqwest::Url;
use rfc5321::Response;

pub fn register<'lua>(lua: &'lua Lua) -> anyhow::Result<()> {
    let xfer_mod = get_or_create_sub_module(lua, "xfer")?;

    xfer_mod.set(
        "get_xfer_target",
        lua.create_async_function(|_lua, msg: UserDataRef<Message>| async move {
            let queue_name = msg.get_queue_name().await.map_err(any_err)?;
            Ok(XferProtocol::from_queue_name(&queue_name).map(|proto| proto.target.to_string()))
        })?,
    )?;

    xfer_mod.set(
        "xfer_in_requeue",
        lua.create_async_function(
            |_lua,
             (msg, target, _insert_context, increment_attempts, delay, reason): (
                UserDataRef<Message>,
                String,
                SerdeWrappedValue<InsertContext>,
                bool,
                Option<UserDataRef<TimeDelta>>,
                Option<String>,
            )| async move {
                let target = XferProtocol {
                    target: Url::parse(&target).map_err(any_err)?,
                };
                let orig_queue_name = msg.get_queue_name().await.map_err(any_err)?;
                let mut sched_queue_name = orig_queue_name.clone();

                match XferProtocol::from_queue_name(&orig_queue_name) {
                    Some(p) => {
                        if p == target {
                            // No change in destination; already xfer'ing
                            // to that location
                            return Ok(());
                        }

                        // Cancel current xfer
                        SavedQueueInfo::restore_info(&msg).await.map_err(any_err)?;
                        // Revise our understanding of which queue this is in
                        sched_queue_name = msg.get_queue_name().await.map_err(any_err)?;
                    }
                    None => {}
                }

                if increment_attempts {
                    let queue = QueueManager::resolve(&sched_queue_name)
                        .await
                        .map_err(any_err)?;
                    queue
                        .increment_attempts_and_update_delay_without_expiry(&msg)
                        .await
                        .map_err(any_err)?;
                } else if let Some(delay) = delay {
                    msg.delay_by((*delay).into()).await.map_err(any_err)?;
                } else {
                    msg.delay_with_jitter(60).await.map_err(any_err)?;
                }

                SavedQueueInfo::save_info(&msg).await.map_err(any_err)?;

                let queue_name = target.to_queue_name();
                msg.set_meta("queue", queue_name.clone())
                    .await
                    .map_err(any_err)?;

                if let Some(reason) = reason {
                    log_disposition(LogDisposition {
                        kind: RecordType::AdminRebind,
                        msg: msg.clone(),
                        site: "",
                        peer_address: None,
                        response: Response {
                            code: 250,
                            enhanced_code: None,
                            command: None,
                            content: format!(
                                "Rebound from {orig_queue_name} to {queue_name}: {reason}"
                            ),
                        },
                        egress_pool: None,
                        egress_source: None,
                        relay_disposition: None,
                        delivery_protocol: None,
                        tls_info: None,
                        source_address: None,
                        provider: None,
                        session_id: None,
                        recipient_list: None,
                    })
                    .await;
                }

                Ok(())
            },
        )?,
    )?;

    xfer_mod.set(
        "xfer",
        lua.create_async_function(
            |_lua,
             (msg, target, reason): (
                UserDataRef<Message>,
                String,
                Option<String>,
            )| async move {
                let target = XferProtocol {
                    target: Url::parse(&target).map_err(any_err)?,
                };
                let orig_queue_name = msg.get_queue_name().await.map_err(any_err)?;
                match XferProtocol::from_queue_name(&orig_queue_name) {
                    Some(p) => {
                        if p == target {
                            // No change in destination; already xfer'ing
                            // to that location
                            return Ok(());
                        }

                        // Cancel current xfer
                        SavedQueueInfo::restore_info(&msg).await.map_err(any_err)?;
                    }
                    None => {}
                }

                SavedQueueInfo::save_info(&msg).await.map_err(any_err)?;

                let queue_name = target.to_queue_name();
                msg.set_meta("queue", queue_name.clone())
                    .await
                    .map_err(any_err)?;

                if let Some(reason) = reason {
                    log_disposition(LogDisposition {
                        kind: RecordType::AdminRebind,
                        msg: msg.clone(),
                        site: "",
                        peer_address: None,
                        response: Response {
                            code: 250,
                            enhanced_code: None,
                            command: None,
                            content: format!(
                                "Rebound from {orig_queue_name} to {queue_name}: {reason}"
                            ),
                        },
                        egress_pool: None,
                        egress_source: None,
                        relay_disposition: None,
                        delivery_protocol: None,
                        tls_info: None,
                        source_address: None,
                        provider: None,
                        session_id: None,
                        recipient_list: None,
                    })
                    .await;
                }

                Ok(())
            },
        )?,
    )?;

    xfer_mod.set(
        "cancel_xfer",
        lua.create_async_function(
            |_lua, (msg, reason): (UserDataRef<Message>, Option<String>)| async move {
                let orig_queue_name = msg.get_queue_name().await.map_err(any_err)?;
                if !XferProtocol::is_xfer_queue_name(&orig_queue_name) {
                    // Nothing to cancel, no need to raise an error
                    return Ok(());
                }

                SavedQueueInfo::restore_info(&msg).await.map_err(any_err)?;
                let queue_name = msg.get_queue_name().await.map_err(any_err)?;

                if let Some(reason) = reason {
                    log_disposition(LogDisposition {
                        kind: RecordType::AdminRebind,
                        msg: msg.clone(),
                        site: "",
                        peer_address: None,
                        response: Response {
                            code: 250,
                            enhanced_code: None,
                            command: None,
                            content: format!(
                                "Rebound from {orig_queue_name} to {queue_name}: {reason}"
                            ),
                        },
                        egress_pool: None,
                        egress_source: None,
                        relay_disposition: None,
                        delivery_protocol: None,
                        tls_info: None,
                        source_address: None,
                        provider: None,
                        session_id: None,
                        recipient_list: None,
                    })
                    .await;
                }

                Ok(())
            },
        )?,
    )?;

    Ok(())
}
