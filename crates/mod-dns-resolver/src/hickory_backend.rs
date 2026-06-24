use crate::config::{
    DnsResolverConfig, IpStrategy, NameServer, Protocol, ResolverOptions, ServerOrderingStrategy,
    UseHostsFile,
};
use anyhow::Context;
use dns_resolver::HickoryResolver;
use hickory_resolver::config::{
    ConnectionConfig, LookupIpStrategy, NameServerConfig, ProtocolConfig, ResolveHosts,
    ResolverConfig, ResolverOpts, ServerOrderingStrategy as HickoryServerOrderingStrategy,
};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::Name;
use hickory_resolver::TokioResolver;
use std::net::SocketAddr;

pub fn build_hickory_resolver(config: &DnsResolverConfig) -> anyhow::Result<HickoryResolver> {
    let mut hickory = ResolverConfig::default();

    if let Some(dom) = &config.domain {
        hickory.set_domain(
            Name::from_str_relaxed(dom.as_str()).with_context(|| format!("domain: '{dom}'"))?,
        );
    }

    for s in &config.search {
        let name = Name::from_str_relaxed(s.as_str()).with_context(|| format!("search: '{s}'"))?;
        hickory.add_search(name);
    }

    for ns in &config.name_servers {
        hickory.add_name_server(translate_name_server(ns)?);
    }

    let mut opts = ResolverOpts::default();
    apply_options(&mut opts, &config.options)?;

    let mut builder = TokioResolver::builder_with_config(hickory, TokioRuntimeProvider::default());
    *builder.options_mut() = opts;
    Ok(HickoryResolver::from(builder.build()?))
}

fn translate_name_server(ns: &NameServer) -> anyhow::Result<NameServerConfig> {
    let (socket_addr, protocol, trust_negative_responses, bind_addr) = match ns {
        NameServer::Ip(s) => (s.as_str(), Protocol::default(), true, None),
        NameServer::Detailed {
            socket_addr,
            protocol,
            trust_negative_responses,
            bind_addr,
        } => (
            socket_addr.as_str(),
            *protocol,
            *trust_negative_responses,
            bind_addr.as_deref(),
        ),
    };

    let sock: SocketAddr = socket_addr
        .parse()
        .with_context(|| format!("name server: '{socket_addr}'"))?;

    let bind_sock = match bind_addr {
        Some(b) => Some(
            b.parse::<SocketAddr>()
                .with_context(|| format!("name server '{socket_addr}' bind_addr: '{b}'"))?,
        ),
        None => None,
    };

    let connections = match protocol {
        Protocol::Udp => vec![build_connection(
            ProtocolConfig::Udp,
            sock.port(),
            bind_sock,
        )],
        Protocol::Tcp => vec![build_connection(
            ProtocolConfig::Tcp,
            sock.port(),
            bind_sock,
        )],
        Protocol::UdpThenTcp => vec![
            build_connection(ProtocolConfig::Udp, sock.port(), bind_sock),
            build_connection(ProtocolConfig::Tcp, sock.port(), bind_sock),
        ],
    };

    Ok(NameServerConfig::new(
        sock.ip(),
        trust_negative_responses,
        connections,
    ))
}

fn build_connection(
    protocol: ProtocolConfig,
    port: u16,
    bind_addr: Option<SocketAddr>,
) -> ConnectionConfig {
    let mut conn = ConnectionConfig::new(protocol);
    conn.port = port;
    conn.bind_addr = bind_addr;
    conn
}

fn apply_options(opts: &mut ResolverOpts, k: &ResolverOptions) -> anyhow::Result<()> {
    if let Some(v) = k.ndots {
        opts.ndots = v;
    }
    if let Some(v) = k.timeout {
        opts.timeout = v;
    }
    if let Some(v) = k.attempts {
        opts.attempts = v;
    }
    if let Some(v) = k.edns0 {
        opts.edns0 = v;
    }
    if let Some(v) = k.validate {
        opts.validate = v;
    }
    if let Some(v) = k.ip_strategy {
        opts.ip_strategy = match v {
            IpStrategy::Ipv4Only => LookupIpStrategy::Ipv4Only,
            IpStrategy::Ipv6Only => LookupIpStrategy::Ipv6Only,
            IpStrategy::Ipv4AndIpv6 => LookupIpStrategy::Ipv4AndIpv6,
            IpStrategy::Ipv6AndIpv4 => LookupIpStrategy::Ipv6AndIpv4,
            IpStrategy::Ipv6thenIpv4 => LookupIpStrategy::Ipv6thenIpv4,
            IpStrategy::Ipv4thenIpv6 => LookupIpStrategy::Ipv4thenIpv6,
        };
    }
    if let Some(v) = k.cache_size {
        opts.cache_size = v;
    }
    if let Some(v) = k.use_hosts_file {
        opts.use_hosts_file = match v {
            UseHostsFile::Always => ResolveHosts::Always,
            UseHostsFile::Auto => ResolveHosts::Auto,
            UseHostsFile::Never => ResolveHosts::Never,
        };
    }
    if let Some(v) = k.positive_min_ttl {
        opts.positive_min_ttl = Some(v);
    }
    if let Some(v) = k.negative_min_ttl {
        opts.negative_min_ttl = Some(v);
    }
    if let Some(v) = k.positive_max_ttl {
        opts.positive_max_ttl = Some(v);
    }
    if let Some(v) = k.negative_max_ttl {
        opts.negative_max_ttl = Some(v);
    }
    if let Some(v) = k.num_concurrent_reqs {
        opts.num_concurrent_reqs = v;
    }
    if let Some(v) = k.preserve_intermediates {
        opts.preserve_intermediates = v;
    }
    if let Some(v) = k.try_tcp_on_error {
        opts.try_tcp_on_error = v;
    }
    if let Some(v) = k.server_ordering_strategy {
        opts.server_ordering_strategy = match v {
            ServerOrderingStrategy::QueryStatistics => {
                HickoryServerOrderingStrategy::QueryStatistics
            }
            ServerOrderingStrategy::RoundRobin => HickoryServerOrderingStrategy::RoundRobin,
            ServerOrderingStrategy::UserProvidedOrder => {
                HickoryServerOrderingStrategy::UserProvidedOrder
            }
        };
    }
    if let Some(v) = k.recursion_desired {
        opts.recursion_desired = v;
    }
    if let Some(v) = k.case_randomization {
        opts.case_randomization = v;
    }
    if let Some(v) = &k.trust_anchor_file {
        opts.trust_anchor = Some(v.clone());
    }
    Ok(())
}
