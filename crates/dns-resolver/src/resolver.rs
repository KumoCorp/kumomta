use hickory_resolver::error::ResolveErrorKind;
use hickory_resolver::proto::op::response_code::ResponseCode;
use hickory_resolver::proto::rr::{DNSClass, RData, RecordType};
use hickory_resolver::{IntoName, TokioAsyncResolver, TryParseIp};
use libunbound::AsyncContext;
use std::net::IpAddr;
use std::time::{Duration, Instant};

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

pub enum Resolver {
    Tokio(TokioAsyncResolver),
    Unbound(AsyncContext),
}

impl Resolver {
    pub async fn resolve_txt<N: IntoName + TryParseIp>(&self, name: N) -> anyhow::Result<Answer> {
        self.resolve(name, RecordType::TXT).await
    }

    pub async fn resolve<N: IntoName + TryParseIp>(
        &self,
        name: N,
        rrtype: RecordType,
    ) -> anyhow::Result<Answer> {
        match self {
            Self::Tokio(t) => match t.lookup(name, rrtype).await {
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
                    _ => Err(err.into()),
                },
            },
            Self::Unbound(ctx) => {
                let name = name.into_name()?;
                let name = name.to_ascii();
                let answer = ctx.resolve(&name, rrtype, DNSClass::IN).await?;
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
    }
}
