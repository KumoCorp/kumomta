use config::{any_err, get_or_create_sub_module, serialize_options, SerdeWrappedValue};
use kumo_dmarc::{Disposition, DmarcPassContext, ReportingInfo};
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
                dkim_results,
                opt_resolver_name,
                spf_result,
                use_reporting,
                opt_reporting_info,
            ): (
                UserDataRef<Message>,
                SerdeWrappedValue<Vec<AuthenticationResult>>,
                Option<String>,
                SerdeWrappedValue<AuthenticationResult>,
                bool,
                Option<SerdeWrappedValue<ReportingInfo>>,
            )| async move {
                let resolver = get_resolver_instance(&opt_resolver_name).map_err(any_err)?;

                // MAIL FROM
                let msg_sender = msg.sender().await;

                let mail_from_domain = msg_sender.ok().map(|x| x.domain().to_string());

                let mut recipient_domain_list = msg
                    .recipient_list()
                    .await
                    .map(|x| {
                        x.into_iter()
                            .map(|x| x.domain().to_string())
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();

                recipient_domain_list.dedup();

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
                                    method: "dmarc".into(),
                                    method_version: None,
                                    result: "None".into(),
                                    reason: Some("'From:' header missing domain".into()),
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
                                method: "dmarc".into(),
                                method_version: None,
                                result: "None".into(),
                                reason: Some("Only single 'From:' header supported".into()),
                                props: BTreeMap::default(),
                            },
                        },
                        serialize_options(),
                    ));
                };

                let dkim_results: Vec<AuthenticationResult> = dkim_results.0;

                let spf_result: AuthenticationResult = spf_result.0;

                let received_from = spf_result
                    .props
                    .get("received_from")
                    .cloned()
                    .unwrap_or_default();

                let reporting_info = if use_reporting {
                    if let Some(reporting_info) = opt_reporting_info {
                        Some(reporting_info.0)
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

                let result = DmarcPassContext {
                    from_domain,
                    mail_from_domain,
                    recipient_domain_list,
                    received_from: received_from.to_string(),
                    dkim_results,
                    spf_result,
                    reporting_info,
                }
                .check(&**resolver)
                .await;

                let disposition = result.result;
                let reason = result.context;
                let mut props = result.props;

                match disposition {
                    Disposition::Pass
                    | Disposition::None
                    | Disposition::TempError
                    | Disposition::PermError => Ok(lua.to_value_with(
                        &CheckHostOutput {
                            disposition,
                            result: AuthenticationResult {
                                method: "dmarc".into(),
                                method_version: None,
                                result: disposition.to_string().to_ascii_lowercase().into(),
                                reason: Some(reason.into()),
                                props,
                            },
                        },
                        serialize_options(),
                    )),
                    disp @ Disposition::Quarantine | disp @ Disposition::Reject => {
                        props.insert(
                            "policy.published-domain-policy".into(),
                            disp.to_string().to_ascii_lowercase().into(),
                        );
                        Ok(lua.to_value_with(
                            &CheckHostOutput {
                                disposition,
                                result: AuthenticationResult {
                                    method: "dmarc".into(),
                                    method_version: None,
                                    result: "fail".into(),
                                    reason: Some(reason.into()),
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
