use crate::error::DnsError;
use futures::future::BoxFuture;
use hickory_resolver::{Name, TokioAsyncResolver};
use std::net::IpAddr;

/// A trait for entities that perform DNS resolution.
pub trait Lookup: Sync + Send {
    fn lookup_ip<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<IpAddr>, DnsError>>;
    fn lookup_mx<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<Name>, DnsError>>;
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, DnsError>>;
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
}
