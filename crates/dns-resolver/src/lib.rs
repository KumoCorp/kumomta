use kumo_log_types::ResolvedAddress;
use lruttl::LruCacheWithTtl;
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use trust_dns_resolver::error::{ResolveErrorKind, ResolveResult};
use trust_dns_resolver::{Name, TokioAsyncResolver};

lazy_static::lazy_static! {
    static ref RESOLVER: TokioAsyncResolver = TokioAsyncResolver::tokio_from_system_conf().unwrap();
    static ref MX_CACHE: StdMutex<LruCacheWithTtl<Name, Arc<MailExchanger>>> = StdMutex::new(LruCacheWithTtl::new(64 * 1024));
    static ref IPV4_CACHE: StdMutex<LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
    static ref IPV6_CACHE: StdMutex<LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
    static ref IP_CACHE: StdMutex<LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
}

#[derive(Clone, Debug, Serialize)]
pub struct MailExchanger {
    pub domain_name: String,
    pub hosts: Vec<String>,
    pub site_name: String,
    pub by_pref: HashMap<u16, Vec<String>>,
}

fn fully_qualify(domain_name: &str) -> ResolveResult<Name> {
    let mut name = Name::from_str_relaxed(domain_name)?.to_lowercase();

    // Treat it as fully qualified
    name.set_fqdn(true);

    Ok(name)
}

impl MailExchanger {
    pub async fn resolve(domain_name: &str) -> anyhow::Result<Arc<Self>> {
        let name_fq = fully_qualify(domain_name)?;
        if let Some(mx) = MX_CACHE.lock().unwrap().get(&name_fq) {
            return Ok(mx);
        }

        let (by_pref, expires) = match lookup_mx_record(&name_fq).await {
            Ok((by_pref, expires)) => (by_pref, expires),
            Err(err) if matches!(err.kind(), ResolveErrorKind::NoRecordsFound { .. }) => {
                match ip_lookup(domain_name).await {
                    Ok((_addr, expires)) => (
                        vec![ByPreference {
                            hosts: vec![name_fq.to_string()],
                            pref: 1,
                        }],
                        expires,
                    ),
                    Err(err) => anyhow::bail!("{err:#}"),
                }
            }
            Err(err) => anyhow::bail!("MX lookup for {domain_name} failed: {err:#}"),
        };

        let mut hosts = vec![];
        for pref in &by_pref {
            for host in &pref.hosts {
                hosts.push(host.to_string());
            }
        }

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
        };

        let mx = Arc::new(mx);
        MX_CACHE
            .lock()
            .unwrap()
            .insert(name_fq, mx.clone(), expires);
        Ok(mx)
    }

    pub async fn resolve_addresses(&self) -> Vec<ResolvedAddress> {
        let mut result = vec![];

        for mx_host in &self.hosts {
            // '.' is a null mx; skip trying to resolve it
            if mx_host == "." {
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
        result
    }
}

struct ByPreference {
    hosts: Vec<String>,
    pref: u16,
}

async fn lookup_mx_record(domain_name: &Name) -> ResolveResult<(Vec<ByPreference>, Instant)> {
    let mx_lookup = RESOLVER.mx_lookup(domain_name.clone()).await?;
    let mx_records = mx_lookup.as_lookup().records();

    if mx_records.is_empty() {
        return Ok((
            vec![ByPreference {
                hosts: vec![domain_name.to_string()],
                pref: 1,
            }],
            mx_lookup.valid_until(),
        ));
    }

    let mut records: Vec<ByPreference> = Vec::with_capacity(mx_records.len());

    for mx_record in mx_records {
        if let Some(mx) = mx_record.data().and_then(|r| r.as_mx()) {
            let pref = mx.preference();
            let host = mx.exchange().to_lowercase().to_string();

            if let Some(record) = records.iter_mut().find(|r| r.pref == pref) {
                record.hosts.push(host);
            } else {
                records.push(ByPreference {
                    hosts: vec![host],
                    pref,
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

    Ok((records, mx_lookup.valid_until()))
}

pub async fn ip_lookup(key: &str) -> ResolveResult<(Arc<Vec<IpAddr>>, Instant)> {
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

pub async fn ipv4_lookup(key: &str) -> ResolveResult<(Arc<Vec<IpAddr>>, Instant)> {
    let key_fq = fully_qualify(key)?;
    if let Some(value) = IPV4_CACHE.lock().unwrap().get_with_expiry(&key_fq) {
        return Ok(value);
    }

    let ipv4_lookup = RESOLVER.ipv4_lookup(key).await?;
    let ips = ipv4_lookup
        .as_lookup()
        .record_iter()
        .filter_map(|r| (IpAddr::from(*r.data()?.as_a()?).into()))
        .collect::<Vec<_>>();

    let ips = Arc::new(ips);
    let expires = ipv4_lookup.valid_until();
    IPV4_CACHE
        .lock()
        .unwrap()
        .insert(key_fq, ips.clone(), expires);
    Ok((ips, expires))
}

pub async fn ipv6_lookup(key: &str) -> ResolveResult<(Arc<Vec<IpAddr>>, Instant)> {
    let key_fq = fully_qualify(key)?;
    if let Some(value) = IPV6_CACHE.lock().unwrap().get_with_expiry(&key_fq) {
        return Ok(value);
    }

    let ipv6_lookup = RESOLVER.ipv6_lookup(key).await?;
    let ips = ipv6_lookup
        .as_lookup()
        .record_iter()
        .filter_map(|r| (IpAddr::from(*r.data()?.as_aaaa()?)).into())
        .collect::<Vec<_>>();

    let ips = Arc::new(ips);
    let expires = ipv6_lookup.valid_until();
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
}
