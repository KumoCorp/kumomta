#![allow(dead_code)]

mod types;

#[cfg(test)]
mod tests;

use crate::types::record::Record;
use crate::types::results::DmarcResultWithContext;
use dns_resolver::Resolver;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::Name;
use std::net::IpAddr;
use std::str::FromStr;
use std::time::SystemTime;

pub use types::results::DmarcResult;

pub struct CheckHostParams {
    /// Domain that provides the sought-after authorization information.
    ///
    /// Initially, the domain portion of the "MAIL FROM" or "HELO" identity.
    pub domain: String,

    /// The "MAIL FROM" email address if available.
    pub sender: Option<String>,

    /// IP address of the SMTP client that is emitting the mail (v4 or v6).
    pub client_ip: IpAddr,
}

impl CheckHostParams {
    pub async fn check(self, resolver: &dyn Resolver) -> DmarcResultWithContext {
        let Self {
            domain,
            sender,
            client_ip,
        } = self;

        let sender = match sender {
            Some(sender) => sender,
            None => format!("postmaster@{domain}"),
        };

        match DmarcContext::new(&sender, &domain, client_ip) {
            Ok(cx) => cx.check(resolver).await,
            Err(result) => result,
        }
    }
}

struct DmarcContext<'a> {
    pub(crate) sender: &'a str,
    pub(crate) local_part: &'a str,
    pub(crate) sender_domain: &'a str,
    pub(crate) domain: &'a str,
    pub(crate) client_ip: IpAddr,
    pub(crate) now: SystemTime,
}

impl<'a> DmarcContext<'a> {
    /// Create a new evaluation context.
    ///
    /// - `sender` is the "MAIL FROM" or "HELO" identity
    /// - `domain` is the domain that provides the sought-after authorization information;
    ///   initially, the domain portion of the "MAIL FROM" or "HELO" identity
    /// - `client_ip` is the IP address of the SMTP client that is emitting the mail
    fn new(
        sender: &'a str,
        domain: &'a str,
        client_ip: IpAddr,
    ) -> Result<Self, DmarcResultWithContext> {
        let Some((local_part, sender_domain)) = sender.split_once('@') else {
            return Err(DmarcResultWithContext {
                result: DmarcResult::Fail,
                context:
                    "input sender parameter '{sender}' is missing @ sign to delimit local part and domain".to_owned(),
            });
        };

        Ok(Self {
            sender,
            local_part,
            sender_domain,
            domain,
            client_ip,
            now: SystemTime::now(),
        })
    }

    pub(crate) fn with_domain(&self, domain: &'a str) -> Self {
        Self { domain, ..*self }
    }

    pub async fn check(&self, resolver: &dyn Resolver) -> DmarcResultWithContext {
        let name = match Name::from_utf8(self.domain) {
            Ok(name) => name,
            Err(_) => {
                return DmarcResultWithContext {
                    result: DmarcResult::Fail,
                    context: format!("invalid domain name: {}", self.domain),
                }
            }
        };

        let initial_txt = match resolver.resolve(name, RecordType::TXT).await {
            Ok(answer) => {
                if answer.records.is_empty() || answer.nxdomain {
                    return DmarcResultWithContext {
                        result: DmarcResult::Fail,
                        context: format!("no DMARC records found for {}", &self.domain),
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
            context: format!("no DMARC records found for {}", &self.domain),
        }
    }
}
