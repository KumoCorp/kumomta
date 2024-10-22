use crate::smtp_server::ConnectionMetaData;
use config::{get_or_create_sub_module, serialize_options};
use kumo_spf::{CheckHostParams, SpfDisposition};
use mailparsing::AuthenticationResult;
use mlua::{Lua, LuaSerdeExt, UserDataRef};
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::str::FromStr;

#[derive(Debug, Serialize)]
struct CheckHostOutput {
    disposition: SpfDisposition,
    result: AuthenticationResult,
}

pub fn register<'lua>(lua: &'lua Lua) -> anyhow::Result<()> {
    let spf_mod = get_or_create_sub_module(lua, "spf")?;

    spf_mod.set(
        "check_host",
        lua.create_async_function(
            |lua, (domain, meta, sender): (String, UserDataRef<ConnectionMetaData>, Option<String>)| async move {
                // `received_from` is always set, and always to a value representing a `SocketAddr`.
                let addr = meta
                    .get_meta("received_from")
                    .and_then(|v| SocketAddr::from_str(v.as_str()?).ok())
                    .unwrap();

                let resolver = dns_resolver::get_resolver();
                let result = CheckHostParams {
                    domain,
                    sender,
                    client_ip: addr.ip(),
                }
                .check(&**resolver)
                .await;

                Ok(lua.to_value_with(
                    &CheckHostOutput {
                        disposition: result.disposition,
                        result: AuthenticationResult {
                            method: "spf".to_string(),
                            method_version: None,
                            result: result.disposition.to_string(),
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
