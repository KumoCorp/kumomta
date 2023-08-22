use anyhow::Context;
use config::{any_err, get_or_create_sub_module};
use dns_resolver::{resolve_a_or_aaaa, MailExchanger};
use mlua::{Lua, LuaSerdeExt};
use std::net::SocketAddr;
use trust_dns_resolver::config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts};
use trust_dns_resolver::{Name, TokioAsyncResolver};

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let dns_mod = get_or_create_sub_module(lua, "dns")?;

    dns_mod.set(
        "lookup_mx",
        lua.create_async_function(|lua, domain: String| async move {
            let mx = MailExchanger::resolve(&domain).await.map_err(any_err)?;
            Ok(lua.to_value(&*mx))
        })?,
    )?;

    dns_mod.set(
        "lookup_addr",
        lua.create_async_function(|_lua, domain: String| async move {
            let result = resolve_a_or_aaaa(&domain).await.map_err(any_err)?;
            let result: Vec<String> = result
                .into_iter()
                .map(|item| item.addr.to_string())
                .collect();
            Ok(result)
        })?,
    )?;

    #[derive(serde::Deserialize, Debug)]
    struct DnsConfig {
        #[serde(default)]
        domain: Option<String>,
        #[serde(default)]
        search: Vec<String>,
        name_servers: Vec<NameServer>,
        #[serde(default)]
        options: ResolverOpts,
    }

    #[derive(serde::Deserialize, Debug)]
    #[serde(untagged)]
    enum NameServer {
        Ip(String),
        Detailed {
            socket_addr: String,
            #[serde(default)]
            protocol: Protocol,
            #[serde(default)]
            trust_negative_responses: bool,
            #[serde(default)]
            bind_addr: Option<String>,
        },
    }

    dns_mod.set(
        "configure_resolver",
        lua.create_async_function(
            |lua, config: mlua::Value| async move {
                let config: DnsConfig = lua.from_value(config)?;

                let mut r_config = ResolverConfig::new();
                if let Some(dom) = config.domain {
                    r_config.set_domain(
                        Name::from_str_relaxed(&dom)
                            .with_context(|| format!("domain: '{dom}'"))
                            .map_err(any_err)?,
                    );
                }
                for s in config.search {
                    let name = Name::from_str_relaxed(&s)
                        .with_context(|| format!("search: '{s}'"))
                        .map_err(any_err)?;
                    r_config.add_search(name);
                }

                for ns in config.name_servers {
                    r_config.add_name_server(match ns {
                        NameServer::Ip(ip) => {
                            let ip: SocketAddr = ip
                                .parse()
                                .with_context(|| format!("name server: '{ip}'"))
                                .map_err(any_err)?;
                            NameServerConfig::new(ip, Protocol::Udp)
                        }
                        NameServer::Detailed {
                            socket_addr,
                            protocol,
                            trust_negative_responses,
                            bind_addr,
                        } => {
                            let ip: SocketAddr = socket_addr
                                .parse()
                                .with_context(|| format!("name server: '{socket_addr}'"))
                                .map_err(any_err)?;
                            let mut c = NameServerConfig::new(ip, protocol);

                            c.trust_nx_responses = trust_negative_responses;

                            if let Some(bind) = bind_addr {
                                let addr: SocketAddr = bind
                                    .parse()
                                    .with_context(|| {
                                        format!("name server: '{socket_addr}' bind_addr: '{bind}'")
                                    })
                                    .map_err(any_err)?;
                                c.bind_addr.replace(addr);
                            }

                            c
                        }
                    });
                }

                let resolver = TokioAsyncResolver::tokio(r_config, config.options).map_err(any_err)?;

                dns_resolver::reconfigure_resolver(resolver).await;

                Ok(())
            },
        )?,
    )?;

    Ok(())
}
