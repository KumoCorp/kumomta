#![allow(dead_code)]

mod types;

#[cfg(test)]
mod tests;

use crate::types::record::Record;
use crate::types::results::DmarcResultWithContext;
use dns_resolver::Resolver;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::Name;
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::time::SystemTime;

pub use types::results::DmarcResult;

pub struct CheckHostParams {
    /// Domain of the sender in the "From:"
    pub from_domain: String,

    /// Domain that provides the sought-after authorization information.
    ///
    /// The "MAIL FROM" email address if available.
    pub mail_from_domain: Option<String>,

    /// IP address of the SMTP client that is emitting the mail (v4 or v6).
    pub client_ip: IpAddr,

    /// The results of the DKIM part of the checks
    pub dkim: Vec<BTreeMap<String, String>>,
}

impl CheckHostParams {
    pub async fn check(self, resolver: &dyn Resolver) -> DmarcResultWithContext {
        let Self {
            from_domain,
            mail_from_domain,
            client_ip,
            dkim,
        } = self;

        match DmarcContext::new(
            &from_domain,
            mail_from_domain.as_ref().map(|x| x.as_str()),
            client_ip,
            &dkim[..],
        ) {
            Ok(cx) => cx.check(resolver).await,
            Err(result) => result,
        }
    }
}

struct DmarcContext<'a> {
    pub(crate) from_domain: &'a str,
    pub(crate) mail_from_domain: Option<&'a str>,
    pub(crate) client_ip: IpAddr,
    pub(crate) now: SystemTime,
    pub(crate) dkim: &'a [BTreeMap<String, String>],
}

impl<'a> DmarcContext<'a> {
    /// Create a new evaluation context.
    ///
    /// - `from_domain` is the domain of the "From:" header
    /// - `mail_from_domain` is the domain portion of the "MAIL FROM" identity
    /// - `client_ip` is the IP address of the SMTP client that is emitting the mail
    fn new(
        from_domain: &'a str,
        mail_from_domain: Option<&'a str>,
        client_ip: IpAddr,
        dkim: &'a [BTreeMap<String, String>],
    ) -> Result<Self, DmarcResultWithContext> {
        Ok(Self {
            from_domain,
            mail_from_domain,
            client_ip,
            now: SystemTime::now(),
            dkim,
        })
    }

    pub async fn check(&self, resolver: &dyn Resolver) -> DmarcResultWithContext {
        let name = match Name::from_utf8(self.from_domain) {
            Ok(name) => name,
            Err(_) => {
                return DmarcResultWithContext {
                    result: DmarcResult::Fail,
                    context: format!("invalid domain name: {}", self.from_domain),
                }
            }
        };

        let initial_txt = match resolver.resolve(name, RecordType::TXT).await {
            Ok(answer) => {
                if answer.records.is_empty() || answer.nxdomain {
                    return DmarcResultWithContext {
                        result: DmarcResult::Fail,
                        context: format!("no DMARC records found for {}", &self.from_domain),
                    };
                } else {
                    answer.as_txt()
                }
            }
            Err(err) => {
                return DmarcResultWithContext {
                    result: DmarcResult::Fail,
                    context: format!("{err}"),
                };
            }
        };

        // TXT records can contain all sorts of stuff, let's walk through
        // the set that we retrieved and take the first one that parses
        for txt in initial_txt {
            if txt.starts_with("v=DMARC1") {
                match Record::from_str(&txt) {
                    Ok(record) => {
                        return record.evaluate(self, resolver).await;
                    }
                    Err(err) => {
                        return DmarcResultWithContext {
                            result: DmarcResult::Fail,
                            context: format!("failed to parse DMARC record: {err}"),
                        };
                    }
                }
            }
        }
        DmarcResultWithContext {
            result: DmarcResult::Fail,
            context: format!("no DMARC records found for {}", &self.from_domain),
        }
    }
}
