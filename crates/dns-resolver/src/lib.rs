use crate::resolver::Resolver;
use arc_swap::ArcSwap;
use hickory_resolver::error::ResolveResult;
pub use hickory_resolver::proto::rr::rdata::tlsa::TLSA;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::Name;
use kumo_log_types::ResolvedAddress;
use lruttl::LruCacheWithTtl;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

pub mod resolver;

lazy_static::lazy_static! {
    static ref RESOLVER: ArcSwap<Resolver> = ArcSwap::from_pointee(default_resolver());
    static ref MX_CACHE: StdMutex<LruCacheWithTtl<Name, Arc<MailExchanger>>> = StdMutex::new(LruCacheWithTtl::new(64 * 1024));
    static ref IPV4_CACHE: StdMutex<LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
    static ref IPV6_CACHE: StdMutex<LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
    static ref IP_CACHE: StdMutex<LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
}

#[cfg(feature = "default-unbound")]
fn default_resolver() -> Resolver {
    // This resolves directly against the root
    let context = libunbound::Context::new().unwrap();
    // and enables DNSSEC
    context.add_builtin_trust_anchors().unwrap();
    Resolver::Unbound(context.into_async().unwrap())
}

#[cfg(not(feature = "default-unbound"))]
fn default_resolver() -> Resolver {
    Resolver::Tokio(hickory_resolver::TokioAsyncResolver::tokio_from_system_conf().unwrap())
}

#[derive(Clone, Debug, Serialize)]
pub struct MailExchanger {
    pub domain_name: String,
    pub hosts: Vec<String>,
    pub site_name: String,
    pub by_pref: BTreeMap<u16, Vec<String>>,
    pub is_domain_literal: bool,
    /// DNSSEC verified
    pub is_secure: bool,
}

pub fn fully_qualify(domain_name: &str) -> ResolveResult<Name> {
    let mut name = Name::from_str_relaxed(domain_name)?.to_lowercase();

    // Treat it as fully qualified
    name.set_fqdn(true);

    Ok(name)
}

pub fn reconfigure_resolver(resolver: Resolver) {
    RESOLVER.store(Arc::new(resolver));
}

pub fn get_resolver() -> Arc<Resolver> {
    RESOLVER.load_full()
}

/// Resolves TLSA records for a destination name and port according to
/// <https://datatracker.ietf.org/doc/html/rfc6698#appendix-B.2>
pub async fn resolve_dane(hostname: &str, port: u16) -> anyhow::Result<Vec<TLSA>> {
    let name = fully_qualify(&format!("_{port}._tcp.{hostname}"))?;
    let answer = RESOLVER.load().resolve(name, RecordType::TLSA).await?;
    tracing::info!("resolve_dane {hostname}:{port} TLSA answer is: {answer:?}");

    if answer.bogus {
        // Bogus records are either tampered with, or due to misconfiguration
        // of the local resolver
        anyhow::bail!(
            "DANE result for {hostname}:{port} unusable because: {}",
            answer
                .why_bogus
                .as_deref()
                .unwrap_or("DNSSEC validation failed")
        );
    }

    let mut result = vec![];
    // We ignore TLSA records unless they are validated; in other words,
    // we'll return an empty list (without raising an error) if the resolver
    // is not configured to verify DNSSEC
    if answer.secure {
        for r in &answer.records {
            if let Some(tlsa) = r.as_tlsa() {
                result.push(tlsa.clone());
            }
        }
        // DNS results are unordered. For the sake of tests,
        // sort these records.
        // Unfortunately, the TLSA type is nor Ord so we
        // convert to string and order by that, which is a bit
        // wasteful but the cardinality of TLSA records is
        // generally low
        result.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
    }

    tracing::info!("resolve_dane {hostname}:{port} result is: {result:?}");

    Ok(result)
}

pub async fn resolve_a_or_aaaa(domain_name: &str) -> anyhow::Result<Vec<ResolvedAddress>> {
    if domain_name.starts_with('[') {
        // It's a literal address, no DNS lookup necessary

        if !domain_name.ends_with(']') {
            anyhow::bail!(
                "domain_name `{domain_name}` is a malformed literal \
                     domain with no trailing `]`"
            );
        }

        let lowered = domain_name.to_ascii_lowercase();
        let literal = &lowered[1..lowered.len() - 1];

        if let Some(v6_literal) = literal.strip_prefix("ipv6:") {
            match v6_literal.parse::<Ipv6Addr>() {
                Ok(addr) => {
                    return Ok(vec![ResolvedAddress {
                        name: domain_name.to_string(),
                        addr: std::net::IpAddr::V6(addr),
                    }]);
                }
                Err(err) => {
                    anyhow::bail!("invalid ipv6 address: `{v6_literal}`: {err:#}");
                }
            }
        }

        // Try to interpret the literal as either an IPv4 or IPv6 address.
        // Note that RFC5321 doesn't actually permit using an untagged
        // IPv6 address, so this is non-conforming behavior.
        match literal.parse::<IpAddr>() {
            Ok(addr) => {
                return Ok(vec![ResolvedAddress {
                    name: domain_name.to_string(),
                    addr,
                }]);
            }
            Err(err) => {
                anyhow::bail!("invalid address: `{literal}`: {err:#}");
            }
        }
    }

    match ip_lookup(domain_name).await {
        Ok((addrs, _expires)) => {
            let addrs = addrs
                .iter()
                .map(|&addr| ResolvedAddress {
                    name: domain_name.to_string(),
                    addr,
                })
                .collect();
            Ok(addrs)
        }
        Err(err) => anyhow::bail!("{err:#}"),
    }
}

impl MailExchanger {
    pub async fn resolve(domain_name: &str) -> anyhow::Result<Arc<Self>> {
        if domain_name.starts_with('[') {
            // It's a literal address, no DNS lookup necessary

            if !domain_name.ends_with(']') {
                anyhow::bail!(
                    "domain_name `{domain_name}` is a malformed literal \
                     domain with no trailing `]`"
                );
            }

            let lowered = domain_name.to_ascii_lowercase();
            let literal = &lowered[1..lowered.len() - 1];

            if let Some(v6_literal) = literal.strip_prefix("ipv6:") {
                match v6_literal.parse::<Ipv6Addr>() {
                    Ok(addr) => {
                        let mut by_pref = BTreeMap::new();
                        by_pref.insert(1, vec![addr.to_string()]);
                        return Ok(Arc::new(Self {
                            domain_name: domain_name.to_string(),
                            hosts: vec![addr.to_string()],
                            site_name: addr.to_string(),
                            by_pref,
                            is_domain_literal: true,
                            is_secure: false,
                        }));
                    }
                    Err(err) => {
                        anyhow::bail!("invalid ipv6 address: `{v6_literal}`: {err:#}");
                    }
                }
            }

            // Try to interpret the literal as either an IPv4 or IPv6 address.
            // Note that RFC5321 doesn't actually permit using an untagged
            // IPv6 address, so this is non-conforming behavior.
            match literal.parse::<IpAddr>() {
                Ok(addr) => {
                    let mut by_pref = BTreeMap::new();
                    by_pref.insert(1, vec![addr.to_string()]);
                    return Ok(Arc::new(Self {
                        domain_name: domain_name.to_string(),
                        hosts: vec![addr.to_string()],
                        site_name: addr.to_string(),
                        by_pref,
                        is_domain_literal: true,
                        is_secure: false,
                    }));
                }
                Err(err) => {
                    anyhow::bail!("invalid address: `{literal}`: {err:#}");
                }
            }
        }

        let name_fq = fully_qualify(domain_name)?;
        if let Some(mx) = MX_CACHE.lock().unwrap().get(&name_fq) {
            return Ok(mx);
        }

        let (by_pref, expires) = match lookup_mx_record(&name_fq).await {
            Ok((by_pref, expires)) => (by_pref, expires),
            Err(err) => anyhow::bail!("MX lookup for {domain_name} failed: {err:#}"),
        };

        let mut hosts = vec![];
        for pref in &by_pref {
            for host in &pref.hosts {
                hosts.push(host.to_string());
            }
        }

        let is_secure = by_pref.iter().all(|p| p.is_secure);

        let by_pref = by_pref
            .into_iter()
            .map(|pref| (pref.pref, pref.hosts))
            .collect();

        let site_name = factor_names(&hosts);
        let mx = Self {
            hosts,
            domain_name: name_fq.to_string(),
            site_name,
            by_pref,
            is_domain_literal: false,
            is_secure,
        };

        let mx = Arc::new(mx);
        MX_CACHE
            .lock()
            .unwrap()
            .insert(name_fq, mx.clone(), expires);
        Ok(mx)
    }

    pub async fn resolve_addresses(&self) -> ResolvedMxAddresses {
        let mut result = vec![];

        for mx_host in &self.hosts {
            // '.' is a null mx; skip trying to resolve it
            if mx_host == "." {
                return ResolvedMxAddresses::NullMx;
            }

            // Handle the literal address case
            if let Ok(addr) = mx_host.parse::<IpAddr>() {
                result.push(ResolvedAddress {
                    name: mx_host.to_string(),
                    addr,
                });
                continue;
            }

            match ip_lookup(mx_host).await {
                Err(err) => {
                    tracing::error!("failed to resolve {mx_host}: {err:#}");
                    continue;
                }
                Ok((addresses, _expires)) => {
                    for addr in addresses.iter() {
                        result.push(ResolvedAddress {
                            name: mx_host.to_string(),
                            addr: *addr,
                        });
                    }
                }
            }
        }
        result.reverse();
        ResolvedMxAddresses::Addresses(result)
    }
}

#[derive(Debug, Clone)]
pub enum ResolvedMxAddresses {
    NullMx,
    Addresses(Vec<ResolvedAddress>),
}

struct ByPreference {
    hosts: Vec<String>,
    pref: u16,
    is_secure: bool,
}

async fn lookup_mx_record(domain_name: &Name) -> anyhow::Result<(Vec<ByPreference>, Instant)> {
    let mx_lookup = RESOLVER
        .load()
        .resolve(domain_name.clone(), RecordType::MX)
        .await?;
    let mx_records = mx_lookup.records;

    if mx_records.is_empty() {
        if mx_lookup.nxdomain {
            anyhow::bail!("NXDOMAIN");
        }

        return Ok((
            vec![ByPreference {
                hosts: vec![domain_name.to_string()],
                pref: 1,
                is_secure: false,
            }],
            mx_lookup.expires,
        ));
    }

    let mut records: Vec<ByPreference> = Vec::with_capacity(mx_records.len());

    for mx_record in mx_records {
        if let Some(mx) = mx_record.as_mx() {
            let pref = mx.preference();
            let host = mx.exchange().to_lowercase().to_string();

            if let Some(record) = records.iter_mut().find(|r| r.pref == pref) {
                record.hosts.push(host);
            } else {
                records.push(ByPreference {
                    hosts: vec![host],
                    pref,
                    is_secure: mx_lookup.secure,
                });
            }
        }
    }

    // Sort by preference
    records.sort_unstable_by(|a, b| a.pref.cmp(&b.pref));

    // Sort the hosts at each preference level to produce the
    // overall ordered list of hosts for this site
    for mx in &mut records {
        mx.hosts.sort();
    }

    Ok((records, mx_lookup.expires))
}

pub async fn ip_lookup(key: &str) -> anyhow::Result<(Arc<Vec<IpAddr>>, Instant)> {
    let key_fq = fully_qualify(key)?;
    if let Some(value) = IP_CACHE.lock().unwrap().get_with_expiry(&key_fq) {
        return Ok(value);
    }

    let (v4, v6) = tokio::join!(ipv4_lookup(key), ipv6_lookup(key));

    let mut results = vec![];
    let mut errors = vec![];
    let mut expires = None;

    match v4 {
        Ok((addrs, exp)) => {
            expires.replace(exp);
            for a in addrs.iter() {
                results.push(*a);
            }
        }
        Err(err) => errors.push(err),
    }

    match v6 {
        Ok((addrs, exp)) => {
            let exp = match expires.take() {
                Some(existing) => exp.min(existing),
                None => exp,
            };
            expires.replace(exp);

            for a in addrs.iter() {
                results.push(*a);
            }
        }
        Err(err) => errors.push(err),
    }

    if results.is_empty() && !errors.is_empty() {
        return Err(errors.remove(0));
    }

    let addr = Arc::new(results);
    let exp = expires.take().unwrap_or_else(|| Instant::now());

    IP_CACHE.lock().unwrap().insert(key_fq, addr.clone(), exp);
    Ok((addr, exp))
}

pub async fn ipv4_lookup(key: &str) -> anyhow::Result<(Arc<Vec<IpAddr>>, Instant)> {
    let key_fq = fully_qualify(key)?;
    if let Some(value) = IPV4_CACHE.lock().unwrap().get_with_expiry(&key_fq) {
        return Ok(value);
    }

    let answer = RESOLVER
        .load()
        .resolve(key_fq.clone(), RecordType::A)
        .await?;
    let ips = answer.as_addr();

    let ips = Arc::new(ips);
    let expires = answer.expires;
    IPV4_CACHE
        .lock()
        .unwrap()
        .insert(key_fq, ips.clone(), expires);
    Ok((ips, expires))
}

pub async fn ipv6_lookup(key: &str) -> anyhow::Result<(Arc<Vec<IpAddr>>, Instant)> {
    let key_fq = fully_qualify(key)?;
    if let Some(value) = IPV6_CACHE.lock().unwrap().get_with_expiry(&key_fq) {
        return Ok(value);
    }

    let answer = RESOLVER
        .load()
        .resolve(key_fq.clone(), RecordType::AAAA)
        .await?;
    let ips = answer.as_addr();

    let ips = Arc::new(ips);
    let expires = answer.expires;
    IPV6_CACHE
        .lock()
        .unwrap()
        .insert(key_fq, ips.clone(), expires);
    Ok((ips, expires))
}

/// Given a list of host names, produce a pseudo-regex style alternation list
/// of the different elements of the hostnames.
/// The goal is to produce a more compact representation of the name list
/// with the common components factored out.
fn factor_names<S: AsRef<str>>(name_strings: &[S]) -> String {
    let mut max_element_count = 0;

    let mut names = vec![];

    for name in name_strings {
        if let Ok(name) = fully_qualify(name.as_ref()) {
            names.push(name.to_lowercase());
        }
    }

    let mut elements: Vec<Vec<&str>> = vec![];

    let mut split_names = vec![];
    for name in names {
        let mut fields: Vec<_> = name
            .iter()
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect();
        fields.reverse();
        max_element_count = max_element_count.max(fields.len());
        split_names.push(fields);
    }

    fn add_element<'a>(elements: &mut Vec<Vec<&'a str>>, field: &'a str, i: usize) {
        match elements.get_mut(i) {
            Some(ele) => {
                if !ele.contains(&field) {
                    ele.push(field);
                }
            }
            None => {
                elements.push(vec![field]);
            }
        }
    }

    for fields in &split_names {
        for (i, field) in fields.iter().enumerate() {
            add_element(&mut elements, field, i);
        }
        for i in fields.len()..max_element_count {
            add_element(&mut elements, "?", i);
        }
    }

    let mut result = vec![];
    for mut ele in elements {
        let has_q = ele.contains(&"?");
        ele.retain(|&e| e != "?");
        let mut item_text = if ele.len() == 1 {
            ele[0].to_string()
        } else {
            format!("({})", ele.join("|"))
        };
        if has_q {
            item_text.push('?');
        }
        result.push(item_text);
    }
    result.reverse();

    result.join(".")
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn literal_resolve() {
        let v4_loopback = MailExchanger::resolve("[127.0.0.1]").await.unwrap();
        k9::snapshot!(
            &v4_loopback,
            r#"
MailExchanger {
    domain_name: "[127.0.0.1]",
    hosts: [
        "127.0.0.1",
    ],
    site_name: "127.0.0.1",
    by_pref: {
        1: [
            "127.0.0.1",
        ],
    },
    is_domain_literal: true,
    is_secure: false,
}
"#
        );
        k9::snapshot!(
            v4_loopback.resolve_addresses().await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "127.0.0.1",
            addr: 127.0.0.1,
        },
    ],
)
"#
        );

        let v6_loopback_non_conforming = MailExchanger::resolve("[::1]").await.unwrap();
        k9::snapshot!(
            &v6_loopback_non_conforming,
            r#"
MailExchanger {
    domain_name: "[::1]",
    hosts: [
        "::1",
    ],
    site_name: "::1",
    by_pref: {
        1: [
            "::1",
        ],
    },
    is_domain_literal: true,
    is_secure: false,
}
"#
        );
        k9::snapshot!(
            v6_loopback_non_conforming.resolve_addresses().await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "::1",
            addr: ::1,
        },
    ],
)
"#
        );

        let v6_loopback = MailExchanger::resolve("[IPv6:::1]").await.unwrap();
        k9::snapshot!(
            &v6_loopback,
            r#"
MailExchanger {
    domain_name: "[IPv6:::1]",
    hosts: [
        "::1",
    ],
    site_name: "::1",
    by_pref: {
        1: [
            "::1",
        ],
    },
    is_domain_literal: true,
    is_secure: false,
}
"#
        );
        k9::snapshot!(
            v6_loopback.resolve_addresses().await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "::1",
            addr: ::1,
        },
    ],
)
"#
        );
    }

    #[test]
    fn name_factoring() {
        assert_eq!(
            factor_names(&[
                "mta5.am0.yahoodns.net",
                "mta6.am0.yahoodns.net",
                "mta7.am0.yahoodns.net"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.net".to_string()
        );

        // Verify that the case is normalized to lowercase
        assert_eq!(
            factor_names(&[
                "mta5.AM0.yahoodns.net",
                "mta6.am0.yAHOodns.net",
                "mta7.am0.yahoodns.net"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.net".to_string()
        );

        // When the names have mismatched lengths, do we produce
        // something reasonable?
        assert_eq!(
            factor_names(&[
                "gmail-smtp-in.l.google.com",
                "alt1.gmail-smtp-in.l.google.com",
                "alt2.gmail-smtp-in.l.google.com",
                "alt3.gmail-smtp-in.l.google.com",
                "alt4.gmail-smtp-in.l.google.com",
            ]),
            "(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com".to_string()
        );
    }

    /// Verify that the order is preserved and that we treat these two
    /// examples of differently ordered sets of the same names as two
    /// separate site name strings
    #[test]
    fn mx_order_name_factor() {
        assert_eq!(
            factor_names(&[
                "example-com.mail.protection.outlook.com.",
                "mx-biz.mail.am0.yahoodns.net.",
                "mx-biz.mail.am0.yahoodns.net.",
            ]),
            "(example-com|mx-biz).mail.(protection|am0).(outlook|yahoodns).(com|net)".to_string()
        );
        assert_eq!(
            factor_names(&[
                "mx-biz.mail.am0.yahoodns.net.",
                "mx-biz.mail.am0.yahoodns.net.",
                "example-com.mail.protection.outlook.com.",
            ]),
            "(mx-biz|example-com).mail.(am0|protection).(yahoodns|outlook).(net|com)".to_string()
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_gmail_mx() {
        let gmail = MailExchanger::resolve("gmail.com").await.unwrap();
        k9::snapshot!(
            gmail,
            r#"
MailExchanger {
    domain_name: "gmail.com.",
    hosts: [
        "gmail-smtp-in.l.google.com.",
        "alt1.gmail-smtp-in.l.google.com.",
        "alt2.gmail-smtp-in.l.google.com.",
        "alt3.gmail-smtp-in.l.google.com.",
        "alt4.gmail-smtp-in.l.google.com.",
    ],
    site_name: "(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com",
    by_pref: {
        5: [
            "gmail-smtp-in.l.google.com.",
        ],
        10: [
            "alt1.gmail-smtp-in.l.google.com.",
        ],
        20: [
            "alt2.gmail-smtp-in.l.google.com.",
        ],
        30: [
            "alt3.gmail-smtp-in.l.google.com.",
        ],
        40: [
            "alt4.gmail-smtp-in.l.google.com.",
        ],
    },
    is_domain_literal: false,
    is_secure: false,
}
"#
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_bogus_aasland() {
        let err = MailExchanger::resolve("not-mairs.aasland.com")
            .await
            .unwrap_err();
        k9::snapshot!(err, "MX lookup for not-mairs.aasland.com failed: NXDOMAIN");
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_example_com() {
        // Has a NULL MX record
        let mx = MailExchanger::resolve("example.com").await.unwrap();
        k9::snapshot!(
            mx,
            r#"
MailExchanger {
    domain_name: "example.com.",
    hosts: [
        ".",
    ],
    site_name: "",
    by_pref: {
        0: [
            ".",
        ],
    },
    is_domain_literal: false,
    is_secure: true,
}
"#
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_have_dane() {
        let mx = MailExchanger::resolve("do.havedane.net").await.unwrap();
        k9::snapshot!(
            mx,
            r#"
MailExchanger {
    domain_name: "do.havedane.net.",
    hosts: [
        "do.havedane.net.",
    ],
    site_name: "do.havedane.net",
    by_pref: {
        10: [
            "do.havedane.net.",
        ],
    },
    is_domain_literal: false,
    is_secure: true,
}
"#
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn tlsa_have_dane() {
        let tlsa = resolve_dane("do.havedane.net", 25).await.unwrap();
        k9::snapshot!(
            tlsa,
            "
[
    TLSA {
        cert_usage: TrustAnchor,
        selector: Spki,
        matching: Sha256,
        cert_data: [
            39,
            182,
            148,
            181,
            29,
            31,
            239,
            136,
            133,
            55,
            42,
            207,
            179,
            145,
            147,
            117,
            151,
            34,
            183,
            54,
            176,
            66,
            104,
            100,
            220,
            28,
            121,
            208,
            101,
            31,
            239,
            115,
        ],
    },
    TLSA {
        cert_usage: DomainIssued,
        selector: Spki,
        matching: Sha256,
        cert_data: [
            85,
            58,
            207,
            136,
            249,
            238,
            24,
            204,
            170,
            230,
            53,
            202,
            84,
            15,
            50,
            203,
            132,
            172,
            167,
            124,
            71,
            145,
            102,
            130,
            188,
            181,
            66,
            213,
            29,
            170,
            135,
            31,
        ],
    },
]
"
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn mx_lookup_www_example_com() {
        // Has no MX, should fall back to A lookup
        let mx = MailExchanger::resolve("www.example.com").await.unwrap();
        k9::snapshot!(
            mx,
            r#"
MailExchanger {
    domain_name: "www.example.com.",
    hosts: [
        "www.example.com.",
    ],
    site_name: "www.example.com",
    by_pref: {
        1: [
            "www.example.com.",
        ],
    },
    is_domain_literal: false,
    is_secure: false,
}
"#
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn txt_lookup_gmail() {
        let answer = get_resolver()
            .resolve("_mta-sts.gmail.com", RecordType::TXT)
            .await
            .unwrap();
        k9::snapshot!(
            answer.as_txt(),
            r#"
[
    "v=STSv1; id=20190429T010101;",
]
"#
        );
    }
}
