#![allow(dead_code)]

use crate::types::record::Record;
pub use crate::types::results::{Disposition, DispositionWithContext};
use dns_resolver::Resolver;
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::time::SystemTime;

pub use types::results::DmarcResult;

mod types;

#[cfg(test)]
mod tests;

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
    pub async fn check(self, resolver: &dyn Resolver) -> DispositionWithContext {
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

pub(crate) enum SenderDomainAlignment {
    /// Sender domain is an exact match to the dmarc record
    Exact,

    /// Sender domain has no exact matching dmarc record
    /// but its organizational domain does
    OrganizationalDomain,
}

pub(crate) enum DmarcRecordResolution {
    /// DNS could not be resolved at this time
    TempError,

    /// DNS was resolved, but no DMARC record was found
    PermError,

    /// DNS was resolved, and DMARC record was found
    Records(Vec<Record>),
}

impl From<DmarcRecordResolution> for Disposition {
    fn from(value: DmarcRecordResolution) -> Self {
        match value {
            DmarcRecordResolution::TempError => Disposition::TempError,
            DmarcRecordResolution::PermError => Disposition::PermError,
            DmarcRecordResolution::Records(_) => {
                panic!("records must be parsed before being used in disposition")
            }
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
    ) -> Result<Self, DispositionWithContext> {
        Ok(Self {
            from_domain,
            mail_from_domain,
            client_ip,
            now: SystemTime::now(),
            dkim,
        })
    }

    pub async fn check(&self, resolver: &dyn Resolver) -> DispositionWithContext {
        match fetch_dmarc_records(&format!("_dmarc.{}", self.from_domain), resolver).await {
            DmarcRecordResolution::Records(records) => {
                for record in records {
                    return record.evaluate(self, SenderDomainAlignment::Exact).await;
                }
            }
            x => {
                if let Some(organizational_domain) = psl::domain_str(self.from_domain) {
                    if organizational_domain != self.from_domain {
                        let address = format!("_dmarc.{}", organizational_domain);
                        match fetch_dmarc_records(&address, resolver).await {
                            DmarcRecordResolution::TempError => {
                                return DispositionWithContext {
                                    result: Disposition::TempError,
                                    context: format!(
                                        "DNS records could not be resolved for {}",
                                        address
                                    ),
                                }
                            }
                            DmarcRecordResolution::PermError => {
                                return DispositionWithContext {
                                    result: Disposition::PermError,
                                    context: format!("no DMARC records found for {}", address),
                                }
                            }
                            DmarcRecordResolution::Records(records) => {
                                for record in records {
                                    return record
                                        .evaluate(self, SenderDomainAlignment::OrganizationalDomain)
                                        .await;
                                }
                            }
                        }
                    } else {
                        return DispositionWithContext {
                            result: x.into(),
                            context: format!("no DMARC records found for {}", &self.from_domain),
                        };
                    }
                }
            }
        }

        DispositionWithContext {
            result: Disposition::None,
            context: format!("no DMARC records found for {}", &self.from_domain),
        }
    }
}

pub(crate) async fn fetch_dmarc_records(
    address: &str,
    resolver: &dyn Resolver,
) -> DmarcRecordResolution {
    let initial_txt = match resolver.resolve_txt(address).await {
        Ok(answer) => {
            if answer.records.is_empty() || answer.nxdomain {
                return DmarcRecordResolution::PermError;
            } else {
                eprintln!("answer: {:?}", answer);
                answer.as_txt()
            }
        }
        Err(_) => {
            return DmarcRecordResolution::TempError;
        }
    };

    let mut records = vec![];

    // TXT records can contain all sorts of stuff, let's walk through
    // the set that we retrieved and take the first one that parses
    for txt in initial_txt {
        if txt.starts_with("v=DMARC1;") {
            if let Ok(record) = Record::from_str(&txt) {
                records.push(record);
            }
        }
    }

    if records.is_empty() {
        return DmarcRecordResolution::PermError;
    }

    DmarcRecordResolution::Records(records)
}
