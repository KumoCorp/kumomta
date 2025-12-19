use config::{any_err, get_or_create_sub_module, serialize_options};
use kumo_dmarc::{CheckHostParams, Disposition, ReportingInfo};
use mailparsing::AuthenticationResult;
use message::Message;
use mlua::{Lua, LuaSerdeExt, UserDataRef};
use mod_dns_resolver::get_resolver_instance;
use serde::Serialize;
use std::collections::BTreeMap;

use crate::smtp_server::{RejectDisconnect, RejectError};

#[derive(Debug, Serialize)]
struct CheckHostOutput {
    disposition: Disposition,
    result: AuthenticationResult,
}

pub fn register<'lua>(lua: &'lua Lua) -> anyhow::Result<()> {
    let dmarc_mod = get_or_create_sub_module(lua, "dmarc")?;

    dmarc_mod.set(
        "check_msg",
        lua.create_async_function(
            |lua,
             (
                msg,
                use_reporting,
                dkim_results,
                spf_result,
                opt_resolver_name,
                opt_reporting_info,
            ): (
                UserDataRef<Message>,
                bool,
                mlua::Value,
                mlua::Value,
                Option<String>,
                mlua::Value,
            )| async move {
                let resolver = get_resolver_instance(&opt_resolver_name).map_err(any_err)?;

                // MAIL FROM
                let msg_sender = msg.sender().await;

                let mail_from_domain = msg_sender.ok().map(|x| x.domain().to_string());

                let recipient_list = msg
                    .recipient_list()
                    .await
                    .map(|x| {
                        x.into_iter()
                            .map(|x| x.domain().to_string())
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();

                // From:
                let from_domain = if let Ok(Some(from)) = msg.get_address_header("From").await {
                    if let Ok(from_domain) = from.domain() {
                        from_domain.to_string()
                    } else {
                        // Handling a missing RFC5322.From domain is outside of dmarc
                        return Ok(lua.to_value_with(
                            &CheckHostOutput {
                                disposition: Disposition::None,
                                result: AuthenticationResult {
                                    method: "dmarc".to_string(),
                                    method_version: None,
                                    result: "None".to_string(),
                                    reason: Some("'From:' header missing domain".to_string()),
                                    props: BTreeMap::default(),
                                },
                            },
                            serialize_options(),
                        ));
                    }
                } else {
                    // The current implementation expects only a single RFC5322.From domain
                    return Ok(lua.to_value_with(
                        &CheckHostOutput {
                            disposition: Disposition::None,
                            result: AuthenticationResult {
                                method: "dmarc".to_string(),
                                method_version: None,
                                result: "None".to_string(),
                                reason: Some("Only single 'From:' header supported".to_string()),
                                props: BTreeMap::default(),
                            },
                        },
                        serialize_options(),
                    ));
                };

                let dkim_results: Vec<AuthenticationResult> =
                    config::from_lua_value(&lua, dkim_results)?;

                let spf_result: AuthenticationResult = config::from_lua_value(&lua, spf_result)?;

                let reporting_info = if use_reporting {
                    if let Ok(reporting_info) =
                        config::from_lua_value::<ReportingInfo>(&lua, opt_reporting_info)
                    {
                        Some(reporting_info)
                    } else {
                        return Err(mlua::Error::external(RejectError {
                            code: 400,
                            message: "DMARC reporting missing required fields".into(),
                            disconnect: RejectDisconnect::If421,
                        }));
                    }
                } else {
                    None
                };

                let result = CheckHostParams {
                    from_domain,
                    mail_from_domain,
                    recipient_list,
                    dkim_results,
                    spf_result,
                    reporting_info,
                }
                .check(&**resolver)
                .await;

                match result.result {
                    Disposition::Pass
                    | Disposition::None
                    | Disposition::TempError
                    | Disposition::PermError => Ok(lua.to_value_with(
                        &CheckHostOutput {
                            disposition: result.result.clone(),
                            result: AuthenticationResult {
                                method: "dmarc".to_string(),
                                method_version: None,
                                result: result.result.to_string().to_ascii_lowercase(),
                                reason: Some(result.context),
                                props: BTreeMap::default(),
                            },
                        },
                        serialize_options(),
                    )),
                    disp @ Disposition::Quarantine | disp @ Disposition::Reject => {
                        let mut props = BTreeMap::default();
                        props.insert(
                            "policy.published-domain-policy".to_string(),
                            disp.to_string().to_ascii_lowercase(),
                        );
                        Ok(lua.to_value_with(
                            &CheckHostOutput {
                                disposition: result.result.clone(),
                                result: AuthenticationResult {
                                    method: "dmarc".to_string(),
                                    method_version: None,
                                    result: "fail".to_string(),
                                    reason: Some(result.context),
                                    props,
                                },
                            },
                            serialize_options(),
                        ))
                    }
                }
            },
        )?,
    )?;

    Ok(())
}
