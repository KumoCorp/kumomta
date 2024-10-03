use futures::future::BoxFuture;
use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use hickory_resolver::proto::rr::rdata::PTR;
use hickory_resolver::proto::rr::{RecordData, RecordType};
use hickory_resolver::{Name, TokioAsyncResolver};
use std::fmt;
use std::net::IpAddr;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DnsError {
    #[error("SPF: DNS record {0} not found")]
    NotFound(String),
    #[error("SPF: {0}")]
    LookupFailed(String),
}

impl DnsError {
    pub(crate) fn from_resolve(name: &str, err: ResolveError) -> Self {
        match err.kind() {
            ResolveErrorKind::NoRecordsFound { .. } => DnsError::NotFound(name.to_string()),
            _ => DnsError::LookupFailed(format!("failed to query DNS for {name}: {err}")),
        }
    }
}

/// A trait for entities that perform DNS resolution.
pub trait Lookup: Sync + Send {
    fn lookup_ip<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<IpAddr>, DnsError>>;
    fn lookup_mx<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<Name>, DnsError>>;
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, DnsError>>;
    fn lookup_ptr<'a>(&'a self, ip: IpAddr) -> BoxFuture<'a, Result<Vec<Name>, DnsError>>;
}

impl Lookup for TokioAsyncResolver {
    fn lookup_ip<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<IpAddr>, DnsError>> {
        Box::pin(async move {
            self.lookup_ip(name)
                .await
                .map_err(|err| DnsError::from_resolve(name, err))?
                .into_iter()
                .map(|ip| Ok(ip))
                .collect()
        })
    }

    fn lookup_mx<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<Name>, DnsError>> {
        Box::pin(async move {
            self.mx_lookup(name)
                .await
                .map_err(|err| DnsError::from_resolve(name, err))?
                .into_iter()
                .map(|mx| Ok(mx.exchange().clone()))
                .collect()
        })
    }

    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, DnsError>> {
        Box::pin(async move {
            self.txt_lookup(name)
                .await
                .map_err(|err| DnsError::from_resolve(name, err))?
                .into_iter()
                .map(|txt| {
                    Ok(txt
                        .iter()
                        .map(|data| String::from_utf8_lossy(data))
                        .collect())
                })
                .collect()
        })
    }

    fn lookup_ptr<'a>(&'a self, ip: IpAddr) -> BoxFuture<'a, Result<Vec<Name>, DnsError>> {
        Box::pin(async move {
            let name = ptr_host(ip);
            self.lookup(name.clone(), RecordType::PTR)
                .await
                .map_err(|err| DnsError::from_resolve(&name, err))?
                .into_iter()
                .map(|ptr| match PTR::try_from_rdata(ptr) {
                    Ok(ptr) => Ok(ptr.0),
                    Err(_) => Err(DnsError::LookupFailed(format!(
                        "invalid record found for PTR record for {ip}"
                    ))),
                })
                .collect()
        })
    }
}

pub(crate) fn ptr_host(ip: IpAddr) -> String {
    let mut out = IpDisplay { ip, reverse: true }.to_string();
    out.push_str(match ip {
        IpAddr::V4(_) => ".in-addr.arpa",
        IpAddr::V6(_) => ".ip6.arpa",
    });
    out
}

pub(crate) struct IpDisplay {
    pub(crate) ip: IpAddr,
    pub(crate) reverse: bool,
}

impl fmt::Display for IpDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ip {
            IpAddr::V4(v4) => {
                let mut bytes = v4.octets();
                if self.reverse {
                    bytes.reverse();
                }
                let mut first = true;
                for byte in bytes {
                    if !first {
                        f.write_str(".")?;
                    }
                    write!(f, "{byte}")?;
                    first = false;
                }
                Ok(())
            }
            IpAddr::V6(v6) => {
                let mut bytes = v6.octets();
                if self.reverse {
                    bytes.reverse();
                }
                let mut first = true;
                for byte in bytes {
                    if !first {
                        f.write_str(".")?;
                    }
                    let (upper, lower) = (byte >> 4, byte & 0xf);
                    if self.reverse {
                        write!(f, "{lower:x}.{upper:x}")?;
                    } else {
                        write!(f, "{upper:x}.{lower:x}")?;
                    }
                    first = false;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ptr_host;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::str::FromStr;

    #[test]
    fn test_ptr_host() {
        assert_eq!(
            ptr_host(Ipv4Addr::new(192, 0, 2, 1).into()),
            "1.2.0.192.in-addr.arpa"
        );
        assert_eq!(
            ptr_host(Ipv6Addr::from_str("2001:db8::1").unwrap().into()),
            "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa"
        );
    }
}
