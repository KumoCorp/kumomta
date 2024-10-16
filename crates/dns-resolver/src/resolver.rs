use async_trait::async_trait;
use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use hickory_resolver::proto::op::response_code::ResponseCode;
#[cfg(feature = "unbound")]
use hickory_resolver::proto::rr::DNSClass;
use hickory_resolver::proto::rr::{RData, RecordType};
use hickory_resolver::{Name, TokioAsyncResolver};
#[cfg(feature = "unbound")]
use libunbound::{AsyncContext, Context};
use std::fmt;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use thiserror::Error;

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

#[derive(Error, Debug)]
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
    async fn resolve_txt(&self, name: &str) -> Result<Answer, DnsError> {
        let name = Name::from_utf8(name)
            .map_err(|err| DnsError::InvalidName(format!("invalid name {name}: {err}")))?;
        self.resolve(name, RecordType::TXT).await
    }

    async fn resolve(&self, name: Name, rrtype: RecordType) -> Result<Answer, DnsError>;
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
