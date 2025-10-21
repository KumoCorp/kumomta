use crate::smtp_server::ConnectionMetaData;
use config::{any_err, get_or_create_sub_module, serialize_options};
use kumo_spf::{CheckHostParams, SpfDisposition};
use mailparsing::AuthenticationResult;
use message::Message;
use mlua::{Lua, LuaSerdeExt, UserDataRef};
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::str::FromStr;

#[derive(Debug, Serialize)]
pub struct CheckHostOutput {
    pub disposition: SpfDisposition,
    pub result: AuthenticationResult,
}

pub fn register<'lua>(lua: &'lua Lua) -> anyhow::Result<()> {
    let spf_mod = get_or_create_sub_module(lua, "spf")?;

    fn build_from_domain_meta_sender(
        domain: String,
        meta: &ConnectionMetaData,
        sender: Option<String>,
    ) -> anyhow::Result<CheckHostParams> {
        let addr: SocketAddr = meta
            .get_meta("received_from")
            .and_then(|v| v.as_str().map(SocketAddr::from_str))
            .transpose()
            .map_err(any_err)?
            .ok_or_else(|| anyhow::anyhow!("received_from missing from metadata"))?;

        let ehlo_domain = if sender.is_some() {
            meta.get_meta_string("ehlo_domain")
        } else {
            Some(domain.clone())
        };
        let relaying_host_name = meta.get_meta_string("hostname");

        Ok(CheckHostParams {
            domain,
            sender,
            client_ip: addr.ip(),
            ehlo_domain,
            relaying_host_name,
        })
    }

    fn build_from_msg(msg: &Message) -> anyhow::Result<CheckHostParams> {
        let addr: SocketAddr = msg
            .get_meta("received_from")
            .map(|v| v.as_str().map(SocketAddr::from_str))
            .map_err(any_err)?
            .transpose()
            .map_err(any_err)?
            .ok_or_else(|| anyhow::anyhow!("received_from missing from metadata"))?;

        let ehlo_domain = msg.get_meta_string("ehlo_domain").map_err(any_err)?;
        let relaying_host_name = msg.get_meta_string("hostname").map_err(any_err)?;

        let sender = msg.sender().map_err(any_err)?;
        let domain = sender.domain().to_string();

        Ok(CheckHostParams {
            domain,
            sender: Some(sender.to_string()),
            client_ip: addr.ip(),
            ehlo_domain,
            relaying_host_name,
        })
    }

    async fn do_check(lua: &mlua::Lua, params: CheckHostParams) -> mlua::Result<mlua::Value> {
        let resolver = dns_resolver::get_resolver();

        let sender = params.sender.clone();
        let result = params.check(&**resolver).await;

        let mut props = BTreeMap::default();

        if let Some(sender) = sender {
            props.insert("smtp.mailfrom".to_string(), sender);
        }

        lua.to_value_with(
            &CheckHostOutput {
                disposition: result.disposition,
                result: AuthenticationResult {
                    method: "spf".to_string(),
                    method_version: None,
                    result: result.disposition.to_string(),
                    reason: Some(result.context),
                    props,
                },
            },
            serialize_options(),
        )
    }

    spf_mod.set(
        "check_host",
        lua.create_async_function(
            |lua, (domain, meta, sender): (String, UserDataRef<ConnectionMetaData>, Option<String>)| async move {
                let params = build_from_domain_meta_sender(domain, &meta, sender).map_err(any_err)?;

                do_check(&lua, params).await

            },
        )?,
    )?;

    spf_mod.set(
        "check_msg",
        lua.create_async_function(|lua, msg: Message| async move {
            let params = build_from_msg(&msg).map_err(any_err)?;
            do_check(&lua, params).await
        })?,
    )?;

    Ok(())
}
