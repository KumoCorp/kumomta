use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::str::FromStr;

use config::{get_or_create_sub_module, serialize_options};
use kumo_dmarc::{CheckHostParams, DmarcResult};
use mailparsing::AuthenticationResult;
use message::Message;
use mlua::{Lua, LuaSerdeExt, UserDataRef};
use serde::Serialize;

use crate::smtp_server::ConnectionMetaData;

#[derive(Debug, Serialize)]
struct CheckHostOutput {
    disposition: DmarcResult,
    result: AuthenticationResult,
}

pub fn register<'lua>(lua: &'lua Lua) -> anyhow::Result<()> {
    let dmarc_mod = get_or_create_sub_module(lua, "dmarc")?;

    dmarc_mod.set(
        "verify",
        lua.create_async_function(
            |lua,
             (msg, dkim_result, meta): (
                UserDataRef<Message>,
                UserDataRef<Vec<AuthenticationResult>>,
                UserDataRef<ConnectionMetaData>,
            )| async move {
                let addr = meta
                    .get_meta("received_from")
                    .and_then(|v| SocketAddr::from_str(v.as_str()?).ok())
                    .expect("`received_from` is always set, and always to a value representing a `SocketAddr`");

                let resolver = dns_resolver::get_resolver();

                // MAIL FROM
                let msg_sender = msg.sender();

                let mail_from_domain = msg_sender.ok().map(|x| x.to_string());

                // From:
                let from_domain = if let Ok(Some(from)) = msg.get_address_header("From") {
                    if let Ok(from_domain) = from.domain() {
                        from_domain.to_string()
                    } else {
                        return Ok(lua.to_value_with(
                            &CheckHostOutput {
                                disposition: DmarcResult::Fail,
                                result: AuthenticationResult {
                                    method: "dmarc".to_string(),
                                    method_version: None,
                                    result: "Only single 'From:' header supported".to_string(),
                                    reason: Some("Only single 'From:' header supported".to_string()),
                                    props: BTreeMap::default(),
                                },
                            },
                            serialize_options(),
                        ))
                    }
                } else {
                    return Ok(lua.to_value_with(
                        &CheckHostOutput {
                            disposition: DmarcResult::Fail,
                            result: AuthenticationResult {
                                method: "dmarc".to_string(),
                                method_version: None,
                                result: "Only single 'From:' header supported".to_string(),
                                reason: Some("Only single 'From:' header supported".to_string()),
                                props: BTreeMap::default(),
                            },
                        },
                        serialize_options(),
                    ))
                };

                let result = CheckHostParams {
                    from_domain,
                    mail_from_domain,
                    client_ip: addr.ip(),
                    dkim: dkim_result.clone().into_iter().map(|x| x.props).collect(),
                }
                .check(&**resolver)
                .await;

                Ok(lua.to_value_with(
                    &CheckHostOutput {
                        disposition: result.result.clone(),
                        result: AuthenticationResult {
                            method: "dmarc".to_string(),
                            method_version: None,
                            result: result.result.to_string(),
                            reason: Some(result.context),
                            props: BTreeMap::default(),
                        },
                    },
                    serialize_options(),
                ))
            },
        )?,
    )?;

    Ok(())
}
