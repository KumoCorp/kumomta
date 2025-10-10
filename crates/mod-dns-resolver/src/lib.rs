use anyhow::Context;
use config::{any_err, get_or_create_sub_module, serialize_options};
use dns_resolver::{
    get_resolver, ptr_host, resolve_a_or_aaaa, reverse_ip, set_mx_concurrency_limit,
    set_mx_negative_cache_ttl, set_mx_timeout, AggregateResolver, HickoryResolver, MailExchanger,
    Resolver, TestResolver, UnboundResolver,
};
use hickory_resolver::config::{NameServerConfig, ResolveHosts, ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::TokioConnectionProvider;
use hickory_resolver::proto::xfer::Protocol;
use hickory_resolver::{Name, TokioResolver};
use kumo_address::host_or_socket::HostOrSocketAddress;
use mlua::{Lua, LuaSerdeExt, Value};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

static RESOLVERS: LazyLock<Mutex<HashMap<String, Arc<Box<dyn Resolver>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let dns_mod = get_or_create_sub_module(lua, "dns")?;

    dns_mod.set(
        "lookup_mx",
        lua.create_async_function(|lua, domain: String| async move {
            let mx = MailExchanger::resolve(&domain).await.map_err(any_err)?;
            Ok(lua.to_value_with(&*mx, serialize_options()))
        })?,
    )?;

    dns_mod.set(
        "set_mx_concurrency_limit",
        lua.create_function(move |_lua, limit: usize| {
            set_mx_concurrency_limit(limit);
            Ok(())
        })?,
    )?;

    dns_mod.set(
        "set_mx_timeout",
        lua.create_function(move |lua, duration: Value| {
            let duration: duration_serde::Wrap<Duration> = lua.from_value(duration)?;
            set_mx_timeout(duration.into_inner()).map_err(any_err)
        })?,
    )?;

    dns_mod.set(
        "set_mx_negative_cache_ttl",
        lua.create_function(move |lua, duration: Value| {
            let duration: duration_serde::Wrap<Duration> = lua.from_value(duration)?;
            set_mx_negative_cache_ttl(duration.into_inner()).map_err(any_err)
        })?,
    )?;

    fn get_resolver_instance(
        opt_resolver_name: &Option<String>,
    ) -> anyhow::Result<Arc<Box<dyn Resolver>>> {
        if let Some(name) = opt_resolver_name {
            return RESOLVERS
                .lock()
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("resolver {name} is not defined"));
        }

        Ok(get_resolver())
    }

    fn get_opt_resolver(
        opt_resolver_name: &Option<String>,
    ) -> anyhow::Result<Option<Arc<Box<dyn Resolver>>>> {
        if let Some(name) = opt_resolver_name {
            let r = RESOLVERS
                .lock()
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("resolver {name} is not defined"))?;
            Ok(Some(r))
        } else {
            Ok(None)
        }
    }

    dns_mod.set(
        "ptr_host",
        lua.create_function(move |_lua, ip: String| {
            let ip: IpAddr = ip.parse().map_err(any_err)?;
            Ok(ptr_host(ip))
        })?,
    )?;

    dns_mod.set(
        "reverse_ip",
        lua.create_function(move |_lua, ip: String| {
            let ip: IpAddr = ip.parse().map_err(any_err)?;
            Ok(reverse_ip(ip))
        })?,
    )?;

    dns_mod.set(
        "rbl_lookup",
        lua.create_async_function(
            |_lua, (ip_str, bl_domain, opt_resolver_name): (String, String, Option<String>)| async move {
                let resolver = get_resolver_instance(&opt_resolver_name).map_err(any_err)?;

                let address: HostOrSocketAddress = ip_str.parse().map_err(any_err)?;
                let reversed_ip = reverse_ip(address.ip().ok_or_else(||mlua::Error::external(format!("{ip_str} is not a valid IpAddr or SocketAddr")))?);
                let name = format!("{reversed_ip}.{bl_domain}.");

                let answers = resolver.resolve_ip(&name).await.map_err(any_err)?;
                match answers.first() {
                    Some(ip) => {
                        let txt = resolver.resolve_txt(&name).await.map(|a| a.as_txt().join("")).ok();
                        Ok((Some(ip.to_string()), txt))
                    }
                    None => {
                        Ok((None, None))
                    }
                }
            },
        )?,
    )?;

    dns_mod.set(
        "lookup_ptr",
        lua.create_async_function(
            |lua, (ip_str, opt_resolver_name): (String, Option<String>)| async move {
                let resolver = get_resolver_instance(&opt_resolver_name).map_err(any_err)?;
                let addr = std::net::IpAddr::from_str(&ip_str).map_err(any_err)?;
                let answer = resolver.resolve_ptr(addr).await.map_err(any_err)?;
                Ok(lua.to_value_with(&*answer, serialize_options()))
            },
        )?,
    )?;

    dns_mod.set(
        "lookup_txt",
        lua.create_async_function(
            |_lua, (domain, opt_resolver_name): (String, Option<String>)| async move {
                let resolver = get_resolver_instance(&opt_resolver_name).map_err(any_err)?;
                let answer = resolver.resolve_txt(&domain).await.map_err(any_err)?;
                Ok(answer.as_txt())
            },
        )?,
    )?;

    dns_mod.set(
        "lookup_addr",
        lua.create_async_function(
            |_lua, (domain, opt_resolver_name): (String, Option<String>)| async move {
                let opt_resolver = get_opt_resolver(&opt_resolver_name).map_err(any_err)?;
                let result = resolve_a_or_aaaa(&domain, opt_resolver.as_ref().map(|r| &***r))
                    .await
                    .map_err(any_err)?;
                let result: Vec<String> = result
                    .into_iter()
                    .map(|item| item.addr.to_string())
                    .collect();
                Ok(result)
            },
        )?,
    )?;

    #[derive(serde::Deserialize, Debug)]
    #[serde(deny_unknown_fields)]
    struct TestResolverConfig {
        zones: Vec<String>,
    }

    impl TestResolverConfig {
        fn make_resolver(&self) -> anyhow::Result<TestResolver> {
            let mut resolver = TestResolver::default();

            for zone in &self.zones {
                resolver = resolver
                    .with_zone(zone)
                    .map_err(|err| anyhow::anyhow!("{err}"))?;
            }

            Ok(resolver)
        }
    }

    #[derive(serde::Deserialize, Debug)]
    enum KumoResolverConfig {
        Hickory(DnsConfig),
        HickorySystemConfig,
        Unbound(DnsConfig),
        Test(TestResolverConfig),
        Aggregate(Vec<KumoResolverConfig>),
    }

    impl KumoResolverConfig {
        fn make_resolver(&self) -> anyhow::Result<Box<dyn Resolver>> {
            match self {
                Self::Hickory(config) => Ok(Box::new(config.make_hickory()?)),
                Self::HickorySystemConfig => Ok(Box::new(HickoryResolver::new()?)),
                Self::Unbound(config) => Ok(Box::new(config.make_unbound()?)),
                Self::Test(config) => Ok(Box::new(config.make_resolver()?)),
                Self::Aggregate(config) => {
                    let mut resolver = AggregateResolver::new();
                    for c in config {
                        resolver.push_resolver(c.make_resolver()?);
                    }
                    Ok(Box::new(resolver))
                }
            }
        }
    }

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

    impl DnsConfig {
        fn make_hickory(&self) -> anyhow::Result<HickoryResolver> {
            let mut config = ResolverConfig::new();
            if let Some(dom) = &self.domain {
                config.set_domain(
                    Name::from_str_relaxed(&dom).with_context(|| format!("domain: '{dom}'"))?,
                );
            }
            for s in &self.search {
                let name = Name::from_str_relaxed(&s).with_context(|| format!("search: '{s}'"))?;
                config.add_search(name);
            }

            for ns in &self.name_servers {
                config.add_name_server(match ns {
                    NameServer::Ip(ip) => {
                        let ip: SocketAddr =
                            ip.parse().with_context(|| format!("name server: '{ip}'"))?;
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
                            .with_context(|| format!("name server: '{socket_addr}'"))?;
                        let mut c = NameServerConfig::new(ip, protocol.clone());

                        c.trust_negative_responses = *trust_negative_responses;

                        if let Some(bind) = bind_addr {
                            let addr: SocketAddr = bind.parse().with_context(|| {
                                format!("name server: '{socket_addr}' bind_addr: '{bind}'")
                            })?;
                            c.bind_addr.replace(addr);
                        }

                        c
                    }
                });
            }

            let mut builder =
                TokioResolver::builder_with_config(config, TokioConnectionProvider::default());
            *builder.options_mut() = self.options.clone();
            Ok(HickoryResolver::from(builder.build()))
        }

        fn make_unbound(&self) -> anyhow::Result<UnboundResolver> {
            let context = libunbound::Context::new()?;

            for ns in &self.name_servers {
                let addr = match ns {
                    NameServer::Ip(ip) => {
                        ip.parse().with_context(|| format!("name server: '{ip}'"))?
                    }
                    NameServer::Detailed { socket_addr, .. } => socket_addr
                        .parse()
                        .with_context(|| format!("name server: '{socket_addr}'"))?,
                };
                context.set_forward(Some(addr)).context("set_forward")?;
            }

            // TODO: expose a way to provide unbound configuration
            // options to this code

            if self.options.validate {
                context
                    .add_builtin_trust_anchors()
                    .context("add_builtin_trust_anchors")?;
            }
            if matches!(
                self.options.use_hosts_file,
                ResolveHosts::Always | ResolveHosts::Auto
            ) {
                context.load_hosts(None).context("load_hosts")?;
            }

            let context = context
                .into_async()
                .context("make async resolver context")?;

            Ok(UnboundResolver::from(context))
        }
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
            match lua.from_value::<KumoResolverConfig>(config.clone()) {
                Ok(config) => {
                    let resolver = config.make_resolver().map_err(any_err)?;
                    dns_resolver::reconfigure_resolver(resolver);
                    Ok(())
                }
                Err(err1) => match lua.from_value::<DnsConfig>(config) {
                    Ok(config) => {
                        let resolver = config.make_hickory().map_err(any_err)?;
                        dns_resolver::reconfigure_resolver(resolver);
                        Ok(())
                    }
                    Err(err2) => {
                        Err(mlua::Error::external(format!("failed to parse config as either KumoResolverConfig ({err1:#}) or DnsConfig ({err2:#})")))
                    }
                }
            }

        })?,
    )?;

    dns_mod.set(
        "define_resolver",
        lua.create_function(move |lua, (name, config): (String, mlua::Value)| {
            let config = lua
                .from_value::<KumoResolverConfig>(config.clone())
                .map_err(any_err)?;
            let resolver = config.make_resolver().map_err(any_err)?;

            RESOLVERS.lock().insert(name, resolver.into());

            Ok(())
        })?,
    )?;

    dns_mod.set(
        "configure_unbound_resolver",
        lua.create_function(move |lua, config: mlua::Value| {
            let config: DnsConfig = lua.from_value(config)?;
            let resolver = config.make_unbound().map_err(any_err)?;
            dns_resolver::reconfigure_resolver(resolver);
            Ok(())
        })?,
    )?;

    dns_mod.set(
        "configure_test_resolver",
        lua.create_function(move |_lua, zones: Vec<String>| {
            let config = TestResolverConfig { zones };
            let resolver = config.make_resolver().map_err(any_err)?;
            dns_resolver::reconfigure_resolver(resolver);
            Ok(())
        })?,
    )?;

    Ok(())
}
