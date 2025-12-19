use config::{any_err, get_or_create_sub_module, serialize_options};
use kumo_dmarc::{CheckHostParams, Disposition};
use mailparsing::AuthenticationResult;
use message::Message;
use mlua::{Lua, LuaSerdeExt, UserDataRef};
use mod_dns_resolver::get_resolver_instance;
use serde::Serialize;
use std::collections::BTreeMap;

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
             (msg, dkim_results, opt_resolver_name): (
                UserDataRef<Message>,
                mlua::Value,
                Option<String>,
            )| async move {
                let resolver = get_resolver_instance(&opt_resolver_name).map_err(any_err)?;

                // MAIL FROM
                let msg_sender = msg.sender().await;

                let mail_from_domain = msg_sender.ok().map(|x| x.domain().to_string());

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

                let result = CheckHostParams {
                    from_domain,
                    mail_from_domain,
                    dkim: dkim_results.clone().into_iter().map(|x| x.props).collect(),
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
