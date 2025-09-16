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
use crate::spf;

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
             (domain, _msg, spf_result, dkim_result, meta): (
                String,
                UserDataRef<Message>,
                UserDataRef<Vec<AuthenticationResult>>,
                UserDataRef<spf::CheckHostOutput>,
                UserDataRef<ConnectionMetaData>,
            )| async move {
                let addr = meta
                    .get_meta("received_from")
                    .and_then(|v| SocketAddr::from_str(v.as_str()?).ok())
                    .expect("`received_from` is always set, and always to a value representing a `SocketAddr`");

                let resolver = dns_resolver::get_resolver();

                let _result = CheckHostParams {
                    domain,
                    sender: None, //FIXME: for now set this to none
                    client_ip: addr.ip(),
                }
                .check(&**resolver)
                .await;

                if (matches!(dkim_result.disposition, kumo_spf::SpfDisposition::Pass)
                    || matches!(dkim_result.disposition, kumo_spf::SpfDisposition::Neutral))
                    && spf_result.iter().all(|x| x.result == "PASS")
                {
                    Ok(lua.to_value_with(
                        &CheckHostOutput {
                            disposition: DmarcResult::Pass,
                            result: AuthenticationResult {
                                method: "dmarc".to_string(),
                                method_version: None,
                                result: "PASS".to_string(),
                                reason: Some("SPF and DKIM pass".to_string()),
                                props: BTreeMap::default(),
                            },
                        },
                        serialize_options(),
                    ))
                } else {
                    Ok(lua.to_value_with(
                        &CheckHostOutput {
                            disposition: DmarcResult::Fail,
                            result: AuthenticationResult {
                                method: "dmarc".to_string(),
                                method_version: None,
                                result: "FAIL".to_string(),
                                reason: Some("SPF and DKIM fail".to_string()),
                                props: BTreeMap::default(),
                            },
                        },
                        serialize_options(),
                    ))
                }
            },
        )?,
    )?;

    Ok(())
}
