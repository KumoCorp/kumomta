#![allow(dead_code)]

use crate::types::date_range::DateRange;
use crate::types::feedback::Feedback;
use crate::types::identifier::Identifier;
use crate::types::policy_published::PolicyPublished;
use crate::types::record::Record;
use crate::types::report_metadata::ReportMetadata;
use crate::types::results::{AuthResults, DmarcResult, PolicyEvaluated, Results, Row};
pub use crate::types::results::{Disposition, DispositionWithContext};
use chrono::Utc;
use dns_resolver::Resolver;
use mailparsing::AuthenticationResult;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;
use std::time::SystemTime;

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

    /// The envelope to
    pub recipient_list: Vec<String>,

    /// The source IP address
    pub received_from: String,

    /// The results of the DKIM part of the checks
    pub dkim_results: Vec<AuthenticationResult>,

    /// The results of the SPF part of the checks
    pub spf_result: AuthenticationResult,

    /// The additional information needed to perform reporting
    pub reporting_info: Option<ReportingInfo>,
}

impl CheckHostParams {
    pub async fn check(self, resolver: &dyn Resolver) -> DispositionWithContext {
        let Self {
            from_domain,
            mail_from_domain,
            recipient_list,
            received_from,
            dkim_results,
            spf_result,
            reporting_info,
        } = self;

        let mut dmarc_context = DmarcContext::new(
            &from_domain,
            mail_from_domain.as_ref().map(|x| x.as_str()),
            &recipient_list[..],
            &received_from,
            &dkim_results[..],
            &spf_result,
            reporting_info.as_ref(),
        );

        dmarc_context.check(resolver).await
    }
}

#[derive(Clone, Copy)]
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
    pub(crate) recipient_list: &'a [String],
    pub(crate) received_from: &'a str,
    pub(crate) now: SystemTime,
    pub(crate) dkim_results: &'a [AuthenticationResult],
    pub(crate) spf_result: &'a AuthenticationResult,
    pub(crate) dkim_aligned: DmarcResult,
    pub(crate) spf_aligned: DmarcResult,
    pub(crate) reporting_info: Option<&'a ReportingInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportingInfo {
    org_name: String,
    email: String,
    extra_contact_info: Option<String>,
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
        recipient_list: &'a [String],
        received_from: &'a str,
        dkim_results: &'a [AuthenticationResult],
        spf_result: &'a AuthenticationResult,
        reporting_info: Option<&'a ReportingInfo>,
    ) -> DmarcContext<'a> {
        Self {
            from_domain,
            mail_from_domain,
            recipient_list,
            received_from,
            now: SystemTime::now(),
            dkim_results,
            spf_result,
            dkim_aligned: DmarcResult::Pass,
            spf_aligned: DmarcResult::Pass,
            reporting_info,
        }
    }

    pub async fn report_error(
        &self,
        record: &Record,
        dmarc_domain: &str,
        sender_domain_alignment: SenderDomainAlignment,
        error: &str,
    ) {
        let Ok(source_ip): Result<IpAddr, _> = self.received_from.parse() else {
            return;
        };

        if let Some(reporting_info) = self.reporting_info {
            let feedback = Feedback::new(
                "1.0".to_string(),
                ReportMetadata::new(
                    reporting_info.org_name.to_string(),
                    reporting_info.email.to_string(),
                    reporting_info.extra_contact_info.to_owned(),
                    uuid::Uuid::new_v4().to_string(),
                    DateRange::new(Utc::now(), Utc::now()),
                    vec![error.to_string()],
                ),
                PolicyPublished::new(
                    dmarc_domain.to_string(),
                    Some(record.align_dkim),
                    Some(record.align_spf),
                    record.policy,
                    record.subdomain_policy.unwrap_or(record.policy),
                    record.rate,
                    record.report_failure,
                ),
                vec![Results {
                    row: Row {
                        source_ip,
                        count: 1,
                        policy_evaluated: PolicyEvaluated {
                            disposition: record.policy_result(sender_domain_alignment),
                            dkim: self.dkim_aligned,
                            spf: self.spf_aligned,
                            reason: vec![],
                        },
                    },
                    identifiers: Identifier {
                        envelope_to: self.recipient_list.into(),
                        envelope_from: if let Some(mail_from_domain) = self.mail_from_domain {
                            vec![mail_from_domain.into()]
                        } else {
                            vec![]
                        },
                        header_from: self.from_domain.into(),
                    },
                    auth_results: AuthResults {
                        dkim: self.dkim_results.iter().map(|x| x.clone().into()).collect(),
                        spf: vec![self.spf_result.clone().into()],
                    },
                }],
            );

            if let Ok(result) = instant_xml::to_string(&feedback) {
                println!("log: {}", result);
            }
        }
    }

    pub async fn check(&mut self, resolver: &dyn Resolver) -> DispositionWithContext {
        let dmarc_domain = format!("_dmarc.{}", self.from_domain);
        match fetch_dmarc_records(&dmarc_domain, resolver).await {
            DmarcRecordResolution::Records(records) => {
                for record in records {
                    return record
                        .evaluate(self, &dmarc_domain, SenderDomainAlignment::Exact)
                        .await;
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
                                        .evaluate(
                                            self,
                                            &address,
                                            SenderDomainAlignment::OrganizationalDomain,
                                        )
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
