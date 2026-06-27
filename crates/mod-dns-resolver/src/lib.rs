use ::config::{any_err, get_or_create_sub_module, serialize_options, SerdeWrappedValue};
use dns_resolver::{
    get_resolver, ptr_host, resolve_a_or_aaaa, reverse_ip, AggregateResolver, HickoryResolver,
    IpLookupStrategy, Resolver, TestResolver,
};
use kumo_address::host_or_socket::HostOrSocketAddress;
use mailexchanger::{
    set_mx_concurrency_limit, set_mx_negative_cache_ttl, set_mx_timeout, MailExchanger,
};
use mlua::{Lua, LuaSerdeExt, Value};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

mod config;
mod hickory_backend;
mod resolv_conf_loader;
mod unbound_backend;

use crate::config::DnsResolverConfig;

static RESOLVERS: LazyLock<Mutex<HashMap<String, Arc<Box<dyn Resolver>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn get_resolver_instance(
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

pub fn get_opt_resolver(
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

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let dns_mod = get_or_create_sub_module(lua, "dns")?;

    dns_mod.set(
        "lookup_mx",
        lua.create_async_function(
            |lua, (domain, opt_resolver_name): (String, Option<String>)| async move {
                let opt_resolver = get_opt_resolver(&opt_resolver_name).map_err(any_err)?;
                let mx = MailExchanger::resolve_via(&domain, opt_resolver.as_ref().map(|r| &***r))
                    .await
                    .map_err(any_err)?;
                Ok(lua.to_value_with(&*mx, serialize_options()))
            },
        )?,
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

    dns_mod.set(
        "set_mta_sts_enabled",
        lua.create_function(move |_lua, enabled: bool| {
            mailexchanger::set_mta_sts_enabled(enabled);
            Ok(())
        })?,
    )?;

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
            |_lua,
             (domain, opt_resolver_name, strategy): (
                String,
                Option<String>,
                Option<SerdeWrappedValue<IpLookupStrategy>>,
            )| async move {
                let opt_resolver = get_opt_resolver(&opt_resolver_name).map_err(any_err)?;
                let result = resolve_a_or_aaaa(
                    &domain,
                    opt_resolver.as_ref().map(|r| &***r),
                    strategy.map(|v| v.0).unwrap_or_default(),
                )
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
    #[serde(untagged)]
    enum ZoneSpec {
        /// An insecure (non-DNSSEC) zone.
        Insecure(String),
        /// A zone with an explicit DNSSEC secure flag.
        Detailed {
            zone: String,
            #[serde(default)]
            secure: bool,
        },
    }

    #[derive(serde::Deserialize, Debug)]
    #[serde(untagged)]
    enum TestResolverConfig {
        /// A bare list of zones (each an insecure string or `{zone, secure}`).
        Zones(Vec<ZoneSpec>),
        /// The full form, which additionally supports forcing SERVFAIL for a
        /// set of owner names.
        Detailed {
            zones: Vec<ZoneSpec>,
            #[serde(default)]
            servfail: Vec<String>,
        },
    }

    impl TestResolverConfig {
        fn make_resolver(&self) -> anyhow::Result<TestResolver> {
            let mut resolver = TestResolver::default();

            let (zones, servfail): (&[ZoneSpec], &[String]) = match self {
                Self::Zones(zones) => (zones, &[]),
                Self::Detailed { zones, servfail } => (zones, servfail),
            };

            for zone in zones {
                resolver = match zone {
                    ZoneSpec::Insecure(zone) => resolver.with_zone(zone),
                    ZoneSpec::Detailed { zone, secure: true } => resolver.with_secure_zone(zone),
                    ZoneSpec::Detailed {
                        zone,
                        secure: false,
                    } => resolver.with_zone(zone),
                }
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            }

            for name in servfail {
                resolver = resolver.with_servfail(name);
            }

            Ok(resolver)
        }
    }

    #[derive(serde::Deserialize, Debug)]
    enum KumoResolverConfig {
        Hickory(DnsResolverConfig),
        HickorySystemConfig,
        Unbound(DnsResolverConfig),
        Test(TestResolverConfig),
        Aggregate(Vec<KumoResolverConfig>),
    }

    impl KumoResolverConfig {
        fn make_resolver(&self, path: &str) -> anyhow::Result<Box<dyn Resolver>> {
            match self {
                Self::Hickory(config) => Ok(Box::new(
                    hickory_backend::build_hickory_resolver(config)
                        .map_err(|e| anyhow::anyhow!("{path}: {e}"))?,
                )),
                Self::HickorySystemConfig => Ok(Box::new(HickoryResolver::new()?)),
                Self::Unbound(config) => Ok(Box::new(
                    unbound_backend::build_unbound_resolver(config)
                        .map_err(|e| anyhow::anyhow!("{path}: {e}"))?,
                )),
                Self::Test(config) => Ok(Box::new(config.make_resolver()?)),
                Self::Aggregate(children) => {
                    let mut resolver = AggregateResolver::new();
                    for (idx, child) in children.iter().enumerate() {
                        let child_path = format!("{path}.Aggregate[{idx}]");
                        resolver.push_resolver(child.make_resolver(&child_path)?);
                    }
                    Ok(Box::new(resolver))
                }
            }
        }
    }

    dns_mod.set(
        "configure_resolver",
        lua.create_function(move |lua, config: mlua::Value| {
            match lua.from_value::<KumoResolverConfig>(config.clone()) {
                Ok(config) => {
                    let resolver = config
                        .make_resolver("configure_resolver")
                        .map_err(any_err)?;
                    dns_resolver::reconfigure_resolver(resolver);
                    Ok(())
                }
                Err(err1) => match lua.from_value::<DnsResolverConfig>(config) {
                    Ok(config) => {
                        let resolver = hickory_backend::build_hickory_resolver(&config)
                            .map_err(any_err)?;
                        dns_resolver::reconfigure_resolver(resolver);
                        Ok(())
                    }
                    Err(err2) => Err(mlua::Error::external(format!(
                        "failed to parse config as either KumoResolverConfig ({err1:#}) or DnsResolverConfig ({err2:#})"
                    ))),
                },
            }
        })?,
    )?;

    dns_mod.set(
        "define_resolver",
        lua.create_function(move |lua, (name, config): (String, mlua::Value)| {
            let config = lua
                .from_value::<KumoResolverConfig>(config.clone())
                .map_err(any_err)?;
            let path = format!("define_resolver({name:?})");
            let resolver = config.make_resolver(&path).map_err(any_err)?;

            RESOLVERS.lock().insert(name, resolver.into());

            Ok(())
        })?,
    )?;

    dns_mod.set(
        "configure_unbound_resolver",
        lua.create_function(move |lua, config: mlua::Value| {
            let config: DnsResolverConfig = lua.from_value(config)?;
            let resolver = unbound_backend::build_unbound_resolver(&config).map_err(any_err)?;
            dns_resolver::reconfigure_resolver(resolver);
            Ok(())
        })?,
    )?;

    dns_mod.set(
        "configure_test_resolver",
        lua.create_function(move |lua, config: mlua::Value| {
            let config = lua
                .from_value::<TestResolverConfig>(config)
                .map_err(any_err)?;
            let resolver = config.make_resolver().map_err(any_err)?;
            dns_resolver::reconfigure_resolver(resolver);
            Ok(())
        })?,
    )?;

    dns_mod.set(
        "configure_test_mta_sts",
        lua.create_function(
            move |_lua, policies: std::collections::BTreeMap<String, String>| {
                let parsed = policies
                    .into_iter()
                    .map(|(domain, text)| {
                        let policy =
                            mta_sts::policy::MtaStsPolicy::parse(&text).map_err(any_err)?;
                        Ok((domain, policy))
                    })
                    .collect::<mlua::Result<std::collections::BTreeMap<_, _>>>()?;
                mta_sts::set_test_policies(parsed);
                Ok(())
            },
        )?,
    )?;

    dns_mod.set(
        "load_resolv_conf",
        lua.create_function(move |lua, path: Option<String>| {
            let config = resolv_conf_loader::load_resolv_conf(path.as_deref()).map_err(any_err)?;
            lua.to_value_with(&config, serialize_options())
        })?,
    )?;

    Ok(())
}
