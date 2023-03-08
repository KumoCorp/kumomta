use lruttl::LruCacheWithTtl;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use trust_dns_resolver::error::{ResolveErrorKind, ResolveResult};
use trust_dns_resolver::TokioAsyncResolver;

lazy_static::lazy_static! {
    static ref RESOLVER: TokioAsyncResolver = TokioAsyncResolver::tokio_from_system_conf().unwrap();
    static ref MX_CACHE: StdMutex<LruCacheWithTtl<String, Arc<MailExchanger>>> = StdMutex::new(LruCacheWithTtl::new(64 * 1024));
    static ref IPV4_CACHE: StdMutex<LruCacheWithTtl<String, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
    static ref IPV6_CACHE: StdMutex<LruCacheWithTtl<String, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
    static ref IP_CACHE: StdMutex<LruCacheWithTtl<String, Arc<Vec<IpAddr>>>> = StdMutex::new(LruCacheWithTtl::new(1024));
}

#[derive(Clone, Debug)]
pub struct MailExchanger {
    pub domain_name: String,
    pub hosts: Vec<String>,
    pub site_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedAddress {
    pub name: String,
    pub addr: IpAddr,
}

impl MailExchanger {
    pub async fn resolve(domain_name: &str) -> anyhow::Result<Arc<Self>> {
        if let Some(mx) = MX_CACHE.lock().unwrap().get(domain_name) {
            return Ok(mx);
        }

        let (hosts, expires) = match lookup_mx_record(domain_name).await {
            Ok((hosts, expires)) if hosts.is_empty() => (vec![domain_name.to_string()], expires),
            Ok((hosts, expires)) => (hosts, expires),
            Err(err) if matches!(err.kind(), ResolveErrorKind::NoRecordsFound { .. }) => {
                match ip_lookup(domain_name).await {
                    Ok((_addr, expires)) => (vec![domain_name.to_string()], expires),
                    Err(err) => anyhow::bail!("{err:#}"),
                }
            }
            Err(err) => anyhow::bail!("MX lookup for {domain_name} failed: {err:#}"),
        };

        let site_name = factor_names(&hosts);
        let mx = Self {
            hosts,
            domain_name: domain_name.to_string(),
            site_name,
        };

        let mx = Arc::new(mx);
        MX_CACHE
            .lock()
            .unwrap()
            .insert(domain_name.to_string(), mx.clone(), expires);
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

async fn lookup_mx_record(domain_name: &str) -> ResolveResult<(Vec<String>, Instant)> {
    let mx_lookup = RESOLVER.mx_lookup(domain_name).await?;
    let mx_records = mx_lookup.as_lookup().records();

    struct ByPreference {
        hosts: Vec<String>,
        pref: u16,
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
    let mut hosts = vec![];
    for mut mx in records {
        mx.hosts.sort();
        hosts.append(&mut mx.hosts);
    }

    Ok((hosts, mx_lookup.valid_until()))
}

pub async fn ip_lookup(key: &str) -> ResolveResult<(Arc<Vec<IpAddr>>, Instant)> {
    if let Some(value) = IP_CACHE.lock().unwrap().get_with_expiry(key) {
        return Ok(value);
    }
    let (addr, exp) = match ipv4_lookup(key).await {
        Ok((v4, exp)) => (v4, exp),
        Err(_) => ipv6_lookup(key).await?,
    };

    IP_CACHE
        .lock()
        .unwrap()
        .insert(key.to_string(), addr.clone(), exp);
    Ok((addr, exp))
}

pub async fn ipv4_lookup(key: &str) -> ResolveResult<(Arc<Vec<IpAddr>>, Instant)> {
    if let Some(value) = IPV4_CACHE.lock().unwrap().get_with_expiry(key) {
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
        .insert(key.to_string(), ips.clone(), expires);
    Ok((ips, expires))
}

pub async fn ipv6_lookup(key: &str) -> ResolveResult<(Arc<Vec<IpAddr>>, Instant)> {
    if let Some(value) = IPV6_CACHE.lock().unwrap().get_with_expiry(key) {
        return Ok(value);
    }

    let ipv6_lookup = RESOLVER.ipv4_lookup(key).await?;
    let ips = ipv6_lookup
        .as_lookup()
        .record_iter()
        .filter_map(|r| (IpAddr::from(*r.data()?.as_a()?)).into())
        .collect::<Vec<_>>();

    let ips = Arc::new(ips);
    let expires = ipv6_lookup.valid_until();
    IPV6_CACHE
        .lock()
        .unwrap()
        .insert(key.to_string(), ips.clone(), expires);
    Ok((ips, expires))
}

/// Given a list of host names, produce a pseudo-regex style alternation list
/// of the different elements of the hostnames.
/// The goal is to produce a more compact representation of the name list
/// with the common components factored out.
fn factor_names<S: AsRef<str>>(names: &[S]) -> String {
    let mut max_element_count = 0;

    let mut elements: Vec<Vec<&str>> = vec![];

    let mut split_names = vec![];
    for name in names {
        let name = name.as_ref();
        let mut fields: Vec<_> = name.split('.').map(|s| s.to_lowercase()).collect();
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
}
