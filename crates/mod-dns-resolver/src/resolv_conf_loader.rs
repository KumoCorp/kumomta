use crate::config::{DnsResolverConfig, NameServer, ResolverOptions};
use anyhow::Context;
use std::path::Path;
use std::time::Duration;

pub fn load_resolv_conf<P: AsRef<Path>>(path: Option<P>) -> anyhow::Result<DnsResolverConfig> {
    let default_path = Path::new("/etc/resolv.conf");
    let path: &Path = match path.as_ref() {
        Some(p) => p.as_ref(),
        None => default_path,
    };
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> anyhow::Result<DnsResolverConfig> {
    let parsed = resolv_conf::Config::parse(bytes)
        .map_err(|e| anyhow::anyhow!("parsing resolv.conf: {e}"))?;

    let mut name_servers = Vec::with_capacity(parsed.nameservers.len());
    for ns in &parsed.nameservers {
        let ip = std::net::IpAddr::from(ns);
        name_servers.push(NameServer::Ip(socket_addr(ip, 53)));
    }

    let domain = parsed.get_system_domain().map(|d| d.as_str().to_string());

    let search: Vec<String> = parsed
        .get_last_search_or_domain()
        .filter(|s| *s != "--")
        .map(|s| s.to_string())
        .collect();

    let options = ResolverOptions {
        ndots: Some(parsed.ndots as usize),
        timeout: Some(Duration::from_secs(parsed.timeout as u64)),
        attempts: Some(parsed.attempts as usize),
        edns0: Some(parsed.edns0),
        ..ResolverOptions::default()
    };

    Ok(DnsResolverConfig {
        domain,
        search,
        name_servers,
        options,
    })
}

fn socket_addr(ip: std::net::IpAddr, port: u16) -> String {
    match ip {
        std::net::IpAddr::V4(v4) => format!("{v4}:{port}"),
        std::net::IpAddr::V6(v6) => format!("[{v6}]:{port}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_ipv4_nameserver() {
        let cfg = parse(b"nameserver 1.2.3.4\n").unwrap();
        assert_eq!(cfg.name_servers.len(), 1);
        match &cfg.name_servers[0] {
            NameServer::Ip(s) => assert_eq!(s, "1.2.3.4:53"),
            _ => panic!("expected Ip variant"),
        }
    }

    #[test]
    fn parses_options_subset() {
        let cfg =
            parse(b"options ndots:5 timeout:7 attempts:4 edns0\nnameserver 1.2.3.4\n").unwrap();
        assert_eq!(cfg.options.ndots, Some(5));
        assert_eq!(cfg.options.timeout, Some(Duration::from_secs(7)));
        assert_eq!(cfg.options.attempts, Some(4));
        assert_eq!(cfg.options.edns0, Some(true));
    }

    #[test]
    fn ipv6_nameserver_uses_bracket_form() {
        let cfg = parse(b"nameserver 2001:db8::1\n").unwrap();
        match &cfg.name_servers[0] {
            NameServer::Ip(s) => assert_eq!(s, "[2001:db8::1]:53"),
            _ => panic!("expected Ip variant"),
        }
    }

    #[test]
    fn search_and_domain() {
        let cfg =
            parse(b"domain example.com\nsearch a.example b.example\nnameserver 1.2.3.4\n").unwrap();
        assert_eq!(cfg.search, vec!["a.example", "b.example"]);
    }
}
