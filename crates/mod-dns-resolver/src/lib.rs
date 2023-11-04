use anyhow::Context;
use config::{any_err, get_or_create_sub_module};
use dns_resolver::resolver::Resolver;
use dns_resolver::{resolve_a_or_aaaa, MailExchanger};
use hickory_resolver::config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts};
use hickory_resolver::{Name, TokioAsyncResolver};
use mlua::{Lua, LuaSerdeExt};
use std::net::SocketAddr;

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
    #[serde(deny_unknown_fields)]
    struct DnsConfig {
        #[serde(default)]
        domain: Option<String>,
        #[serde(default)]
        search: Vec<String>,
        #[serde(default)]
        name_servers: Vec<NameServer>,
        #[serde(default)]
        options: ResolverOpts,
    }

    #[derive(serde::Deserialize, Debug)]
    #[serde(untagged)]
    #[serde(deny_unknown_fields)]
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
        lua.create_function(move |lua, config: mlua::Value| {
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

                        c.trust_negative_responses = trust_negative_responses;

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

            let resolver = TokioAsyncResolver::tokio(r_config, config.options);

            dns_resolver::reconfigure_resolver(Resolver::Tokio(resolver));

            Ok(())
        })?,
    )?;

    dns_mod.set(
        "configure_unbound_resolver",
        lua.create_function(move |lua, config: mlua::Value| {
            let config: DnsConfig = lua.from_value(config)?;

            let context = libunbound::Context::new().map_err(any_err)?;

            for ns in config.name_servers {
                let addr = match ns {
                    NameServer::Ip(ip) => {
                        let ip: SocketAddr = ip
                            .parse()
                            .with_context(|| format!("name server: '{ip}'"))
                            .map_err(any_err)?;
                        ip.ip()
                    }
                    NameServer::Detailed { socket_addr, .. } => {
                        let ip: SocketAddr = socket_addr
                            .parse()
                            .with_context(|| format!("name server: '{socket_addr}'"))
                            .map_err(any_err)?;
                        ip.ip()
                    }
                };
                context
                    .set_forward(Some(addr))
                    .context("set_forward")
                    .map_err(any_err)?;
            }

            // TODO: expose a way to provide unbound configuration
            // options to this code

            if config.options.validate {
                context
                    .add_builtin_trust_anchors()
                    .context("add_builtin_trust_anchors")
                    .map_err(any_err)?;
            }
            if config.options.use_hosts_file {
                context
                    .load_hosts(None)
                    .context("load_hosts")
                    .map_err(any_err)?;
            }

            let context = context
                .into_async()
                .context("make async resolver context")
                .map_err(any_err)?;

            dns_resolver::reconfigure_resolver(Resolver::Unbound(context));

            Ok(())
        })?,
    )?;

    Ok(())
}
