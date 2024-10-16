use async_trait::async_trait;
use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use hickory_resolver::proto::op::response_code::ResponseCode;
use hickory_resolver::proto::rr::rdata::{A, AAAA, MX, PTR, TXT};
#[cfg(feature = "unbound")]
use hickory_resolver::proto::rr::DNSClass;
use hickory_resolver::proto::rr::{LowerName, RData, RecordData, RecordSet, RecordType, RrKey};
use hickory_resolver::proto::serialize::txt::Parser;
use hickory_resolver::{Name, TokioAsyncResolver};
#[cfg(feature = "unbound")]
use libunbound::{AsyncContext, Context};
use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;
use std::time::{Duration, Instant};
use thiserror::Error;

pub struct IpDisplay {
    pub ip: IpAddr,
    pub reverse: bool,
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

pub fn ptr_host(ip: IpAddr) -> String {
    let mut out = IpDisplay { ip, reverse: true }.to_string();
    out.push_str(match ip {
        IpAddr::V4(_) => ".in-addr.arpa",
        IpAddr::V6(_) => ".ip6.arpa",
    });
    out
}

#[derive(Debug)]
pub struct Answer {
    pub canon_name: Option<String>,
    pub records: Vec<RData>,
    pub nxdomain: bool,
    pub secure: bool,
    pub bogus: bool,
    pub why_bogus: Option<String>,
    pub expires: Instant,
    pub response_code: ResponseCode,
}

impl Answer {
    pub fn as_txt(&self) -> Vec<String> {
        let mut result = vec![];
        for r in &self.records {
            if let Some(txt) = r.as_txt() {
                for t in txt.iter() {
                    result.push(String::from_utf8_lossy(t).to_string());
                }
            }
        }
        result
    }

    pub fn as_addr(&self) -> Vec<IpAddr> {
        let mut result = vec![];
        for r in &self.records {
            if let Some(a) = r.as_a() {
                result.push(a.0.into());
            } else if let Some(a) = r.as_aaaa() {
                result.push(a.0.into());
            }
        }
        result
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum DnsError {
    #[error("invalid DNS name: {0}")]
    InvalidName(String),
    #[error("DNS: {0}")]
    ResolveFailed(String),
}

impl DnsError {
    pub(crate) fn from_resolve(name: &impl fmt::Display, err: ResolveError) -> Self {
        DnsError::ResolveFailed(format!("failed to query DNS for {name}: {err}"))
    }
}

#[async_trait]
pub trait Resolver: Send + Sync + 'static {
    async fn resolve_ip(&self, host: &str) -> Result<Vec<IpAddr>, DnsError>;

    async fn resolve_mx(&self, host: &str) -> Result<Vec<Name>, DnsError>;

    async fn resolve_ptr(&self, ip: IpAddr) -> Result<Vec<Name>, DnsError>;

    async fn resolve_txt(&self, name: &str) -> Result<Answer, DnsError> {
        let name = Name::from_utf8(name)
            .map_err(|err| DnsError::InvalidName(format!("invalid name {name}: {err}")))?;
        self.resolve(name, RecordType::TXT).await
    }

    async fn resolve(&self, name: Name, rrtype: RecordType) -> Result<Answer, DnsError>;
}

#[derive(Default)]
pub struct TestResolver {
    records: BTreeMap<Name, BTreeMap<RrKey, RecordSet>>,
}

impl TestResolver {
    pub fn with_zone(mut self, zone: &str) -> Self {
        let (name, records) = Parser::new(zone, None, None).parse().unwrap();
        self.records.insert(name, records);
        self
    }

    pub fn with_txt(mut self, domain: &str, value: String) -> Self {
        let fqdn = format!("{}.", domain);
        let authority = Name::from_str(&fqdn).unwrap();
        let key = RrKey {
            name: LowerName::from_str(&fqdn).unwrap(),
            record_type: RecordType::TXT,
        };

        let mut records = RecordSet::new(&authority, RecordType::TXT, 0);
        records.add_rdata(RData::TXT(TXT::new(vec![value])));
        self.records
            .entry(authority)
            .or_insert_with(BTreeMap::new)
            .insert(key, records);

        self
    }

    fn get<'a>(
        &'a self,
        full: &str,
        record_type: RecordType,
    ) -> Result<Option<&'a RecordSet>, DnsError> {
        let mut authority = full;
        loop {
            let authority_name = Name::from_utf8(authority).unwrap();
            let Some(records) = self.records.get(&authority_name) else {
                match authority.split_once('.') {
                    Some(new) => {
                        authority = new.1;
                        continue;
                    }
                    None => {
                        println!("authority not found: {full}");
                        return Err(DnsError::ResolveFailed(format!(
                            "authority not found: {full}"
                        )));
                    }
                }
            };

            let fqdn = match full.ends_with('.') {
                true => full,
                false => &format!("{}.", full),
            };

            return Ok(records.get(&RrKey {
                name: LowerName::from_str(&fqdn).unwrap(),
                record_type,
            }));
        }
    }
}

#[async_trait]
impl Resolver for TestResolver {
    async fn resolve_ip(&self, full: &str) -> Result<Vec<IpAddr>, DnsError> {
        let mut values = vec![];

        if let Some(records) = self.get(full, RecordType::A)? {
            for record in records.records_without_rrsigs() {
                let a = A::try_borrow(record.data().unwrap()).unwrap();
                values.push(IpAddr::V4(a.0));
            }
        };

        if let Some(records) = self.get(full, RecordType::AAAA)? {
            for record in records.records_without_rrsigs() {
                let a = AAAA::try_borrow(record.data().unwrap()).unwrap();
                values.push(IpAddr::V6(a.0));
            }
        }

        Ok(values)
    }

    async fn resolve_mx(&self, full: &str) -> Result<Vec<Name>, DnsError> {
        let records = match self.get(full, RecordType::MX)? {
            Some(records) => records,
            None => {
                println!("key not found: {full}");
                return Ok(vec![]);
            }
        };

        let mut values = vec![];
        for record in records.records_without_rrsigs() {
            let mx = MX::try_borrow(record.data().unwrap()).unwrap();
            values.push(mx.exchange().clone());
        }

        Ok(values)
    }

    async fn resolve_txt(&self, full: &str) -> Result<Answer, DnsError> {
        let set = match self.get(full, RecordType::TXT)? {
            Some(records) => records,
            None => {
                println!("key not found: {full}");
                return Ok(Answer {
                    canon_name: None,
                    records: vec![],
                    nxdomain: true,
                    secure: false,
                    bogus: false,
                    why_bogus: None,
                    expires: Instant::now() + Duration::from_secs(60),
                    response_code: ResponseCode::NXDomain,
                });
            }
        };

        let mut records = vec![];
        for record in set.records_without_rrsigs() {
            records.push(record.data().unwrap().clone());
        }

        Ok(Answer {
            canon_name: None,
            records,
            nxdomain: false,
            secure: false,
            bogus: false,
            why_bogus: None,
            expires: Instant::now() + Duration::from_secs(60),
            response_code: ResponseCode::NoError,
        })
    }

    async fn resolve_ptr(&self, ip: IpAddr) -> Result<Vec<Name>, DnsError> {
        let name = ptr_host(ip);

        let records = match self.get(&name, RecordType::PTR)? {
            Some(records) => records,
            None => {
                println!("key not found: {name}");
                return Ok(vec![]);
            }
        };

        let mut values = vec![];
        for record in records.records_without_rrsigs() {
            match PTR::try_borrow(record.data().unwrap()) {
                Some(ptr) => values.push(ptr.0.clone()),
                None => {
                    println!("invalid record found for PTR record for {ip}");
                    return Err(DnsError::ResolveFailed(format!(
                        "invalid record found for PTR record for {ip}"
                    )));
                }
            };
        }

        Ok(values)
    }

    async fn resolve(&self, name: Name, rrtype: RecordType) -> Result<Answer, DnsError> {
        let records = match self.records.get(&name) {
            Some(records) => records,
            None => {
                println!("key not found: {name}");
                return Ok(Answer {
                    canon_name: None,
                    records: vec![],
                    nxdomain: true,
                    secure: false,
                    bogus: false,
                    why_bogus: None,
                    expires: Instant::now() + Duration::from_secs(60),
                    response_code: ResponseCode::NXDomain,
                });
            }
        };

        let mut values = vec![];
        for record in records.values() {
            if record.record_type() == rrtype {
                for record in record.records_without_rrsigs() {
                    values.push(record.data().unwrap().clone());
                }
            }
        }

        Ok(Answer {
            canon_name: None,
            records: values,
            nxdomain: false,
            secure: false,
            bogus: false,
            why_bogus: None,
            expires: Instant::now() + Duration::from_secs(60),
            response_code: ResponseCode::NoError,
        })
    }
}

#[cfg(feature = "unbound")]
pub struct UnboundResolver {
    cx: AsyncContext,
}

#[cfg(feature = "unbound")]
impl UnboundResolver {
    pub fn new() -> Result<Self, libunbound::Error> {
        // This resolves directly against the root
        let context = Context::new()?;
        // and enables DNSSEC
        context.add_builtin_trust_anchors()?;
        Ok(Self {
            cx: context.into_async()?,
        })
    }
}

#[cfg(feature = "unbound")]
#[async_trait]
impl Resolver for UnboundResolver {
    async fn resolve_ip(&self, host: &str) -> Result<Vec<IpAddr>, DnsError> {
        let (a, aaaa) = tokio::join!(
            self.cx.resolve(host, RecordType::A, DNSClass::IN),
            self.cx.resolve(host, RecordType::AAAA, DNSClass::IN),
        );

        let mut records = vec![];
        match (a, aaaa) {
            (Ok(a), Ok(aaaa)) => {
                records.extend(a.rdata().filter_map(|r| match r {
                    Ok(r) => r.as_a().map(|a| IpAddr::from(a.0)),
                    Err(_) => None,
                }));
                records.extend(aaaa.rdata().filter_map(|r| match r {
                    Ok(r) => r.as_aaaa().map(|aaaa| IpAddr::from(aaaa.0)),
                    Err(_) => None,
                }));
            }
            (Ok(a), Err(_)) => {
                records.extend(a.rdata().filter_map(|r| match r {
                    Ok(r) => r.as_a().map(|a| IpAddr::from(a.0)),
                    Err(_) => None,
                }));
            }
            (Err(_), Ok(aaaa)) => {
                records.extend(aaaa.rdata().filter_map(|r| match r {
                    Ok(r) => r.as_aaaa().map(|aaaa| IpAddr::from(aaaa.0)),
                    Err(_) => None,
                }));
            }
            (Err(err), Err(_)) => {
                return Err(DnsError::ResolveFailed(format!(
                    "failed to query DNS for {host}: {err}"
                )))
            }
        }

        Ok(records)
    }

    async fn resolve_mx(&self, host: &str) -> Result<Vec<Name>, DnsError> {
        let answer = self
            .cx
            .resolve(host, RecordType::A, DNSClass::IN)
            .await
            .map_err(|err| {
                DnsError::ResolveFailed(format!("failed to query DNS for {host}: {err}"))
            })?;

        Ok(answer
            .rdata()
            .filter_map(|r| match r {
                Ok(r) => r.as_mx().map(|mx| mx.exchange().clone()),
                Err(_) => None,
            })
            .collect())
    }

    async fn resolve_ptr(&self, ip: IpAddr) -> Result<Vec<Name>, DnsError> {
        let name = ptr_host(ip);
        let answer = self
            .cx
            .resolve(&name, RecordType::PTR, DNSClass::IN)
            .await
            .map_err(|err| {
                DnsError::ResolveFailed(format!("failed to query DNS for {name}: {err}"))
            })?;

        Ok(answer
            .rdata()
            .filter_map(|r| match r {
                Ok(r) => r.as_ptr().map(|ptr| ptr.0.clone()),
                Err(_) => None,
            })
            .collect())
    }

    async fn resolve(&self, name: Name, rrtype: RecordType) -> Result<Answer, DnsError> {
        let name = name.to_ascii();
        let answer = self
            .cx
            .resolve(&name, rrtype, DNSClass::IN)
            .await
            .map_err(|err| {
                DnsError::ResolveFailed(format!("failed to query DNS for {name}: {err}"))
            })?;

        let mut records = vec![];
        for r in answer.rdata() {
            if let Ok(r) = r {
                records.push(r);
            }
        }

        Ok(Answer {
            canon_name: answer.canon_name().map(|s| s.to_string()),
            records,
            nxdomain: answer.nxdomain(),
            secure: answer.secure(),
            bogus: answer.bogus(),
            why_bogus: answer.why_bogus().map(|s| s.to_string()),
            response_code: answer.rcode(),
            expires: Instant::now() + Duration::from_secs(answer.ttl() as u64),
        })
    }
}

#[cfg(feature = "unbound")]
impl From<AsyncContext> for UnboundResolver {
    fn from(cx: AsyncContext) -> Self {
        Self { cx }
    }
}

pub struct HickoryResolver {
    inner: TokioAsyncResolver,
}

impl HickoryResolver {
    pub fn new() -> Result<Self, hickory_resolver::error::ResolveError> {
        Ok(Self {
            inner: TokioAsyncResolver::tokio_from_system_conf()?,
        })
    }
}

#[async_trait]
impl Resolver for HickoryResolver {
    async fn resolve_ip(&self, host: &str) -> Result<Vec<IpAddr>, DnsError> {
        let name = Name::from_utf8(host)
            .map_err(|err| DnsError::InvalidName(format!("invalid name {host}: {err}")))?;

        self.inner
            .lookup_ip(name)
            .await
            .map_err(|err| DnsError::from_resolve(&host, err))?
            .into_iter()
            .map(|ip| Ok(ip))
            .collect()
    }

    async fn resolve_mx(&self, host: &str) -> Result<Vec<Name>, DnsError> {
        let name = Name::from_utf8(host)
            .map_err(|err| DnsError::InvalidName(format!("invalid name {host}: {err}")))?;

        self.inner
            .mx_lookup(name)
            .await
            .map_err(|err| DnsError::from_resolve(&host, err))?
            .into_iter()
            .map(|mx| Ok(mx.exchange().clone()))
            .collect()
    }

    async fn resolve_ptr(&self, ip: IpAddr) -> Result<Vec<Name>, DnsError> {
        self.inner
            .reverse_lookup(ip)
            .await
            .map_err(|err| DnsError::from_resolve(&ip, err))?
            .into_iter()
            .map(|ptr| Ok(ptr.0))
            .collect()
    }

    async fn resolve(&self, name: Name, rrtype: RecordType) -> Result<Answer, DnsError> {
        match self.inner.lookup(name.clone(), rrtype).await {
            Ok(result) => {
                let expires = result.valid_until();
                let records = result.iter().cloned().collect();
                Ok(Answer {
                    canon_name: None,
                    records,
                    nxdomain: false,
                    secure: false,
                    bogus: false,
                    why_bogus: None,
                    expires,
                    response_code: ResponseCode::NoError,
                })
            }
            Err(err) => match err.kind() {
                ResolveErrorKind::NoRecordsFound {
                    negative_ttl,
                    response_code,
                    ..
                } => Ok(Answer {
                    canon_name: None,
                    records: vec![],
                    nxdomain: *response_code == ResponseCode::NXDomain,
                    secure: false,
                    bogus: false,
                    why_bogus: None,
                    response_code: *response_code,
                    expires: Instant::now()
                        + Duration::from_secs(negative_ttl.unwrap_or(60) as u64),
                }),
                _ => Err(DnsError::from_resolve(&name, err)),
            },
        }
    }
}

impl From<TokioAsyncResolver> for HickoryResolver {
    fn from(inner: TokioAsyncResolver) -> Self {
        Self { inner }
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
