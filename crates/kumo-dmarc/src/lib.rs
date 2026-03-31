#![allow(dead_code)]

use crate::types::date_range::DateRange;
use crate::types::feedback::Feedback;
use crate::types::identifier::Identifier;
use crate::types::mode::Mode;
use crate::types::policy::Policy;
use crate::types::policy_published::PolicyPublished;
use crate::types::record::Record;
use crate::types::report_failure::ReportFailure;
use crate::types::report_metadata::ReportMetadata;
use crate::types::results::{AuthResults, DmarcResult, PolicyEvaluated, Results, Row};
pub use crate::types::results::{Disposition, DispositionWithContext};
use chrono::{DateTime, Utc};
use dns_resolver::Resolver;
use mailparsing::AuthenticationResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::time::SystemTime;
use uuid::Uuid;

mod types;

#[cfg(test)]
mod tests;

const DMARC_REPORT_LOG_FILEPATH: &'static str = "/var/log/kumomta/dmarc.log";

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
            received_from.as_str(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportingInfo {
    org_name: String,
    email: String,
    extra_contact_info: Option<String>,
}

/// The individual error records that are then aggregated for output in the report
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct ErrorRecord {
    pub(crate) version: String,
    pub(crate) org_name: String,
    pub(crate) email: String,
    pub(crate) extra_contact_info: Option<String>,
    pub(crate) when: DateTime<Utc>,
    pub(crate) error: String,
    pub(crate) domain: String,
    pub(crate) align_dkim: Option<Mode>,
    pub(crate) align_spf: Option<Mode>,
    pub(crate) policy: Policy,
    pub(crate) subdomain_policy: Policy,
    pub(crate) rate: u8,
    pub(crate) report_failure: ReportFailure,
    pub(crate) source_ip: IpAddr,
    pub(crate) policy_evaluated: PolicyEvaluated,
    pub(crate) identifier: Identifier,
    pub(crate) auth_results: AuthResults,
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
    ) -> std::io::Result<()> {
        let source_ip = match self.received_from.parse() {
            Ok(source_ip) => source_ip,
            Err(_) => Ipv4Addr::new(127, 0, 0, 1).into(),
        };

        if let Some(reporting_info) = self.reporting_info {
            let error_record = ErrorRecord {
                version: "1.0".to_string(),
                org_name: reporting_info.org_name.to_string(),
                email: reporting_info.email.to_string(),
                extra_contact_info: reporting_info.extra_contact_info.to_owned(),
                when: Utc::now(),
                error: error.to_string(),
                domain: dmarc_domain.to_string(),
                align_dkim: Some(record.align_dkim),
                align_spf: Some(record.align_spf),
                policy: record.policy,
                subdomain_policy: record.subdomain_policy.unwrap_or(record.policy),
                rate: record.rate,
                report_failure: record.report_failure,
                source_ip,
                policy_evaluated: PolicyEvaluated {
                    disposition: record.policy_result(sender_domain_alignment),
                    dkim: self.dkim_aligned,
                    spf: self.spf_aligned,
                    reason: vec![],
                },
                identifier: Identifier {
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
            };

            let result = serde_json::to_string(&error_record)?;

            let mut f = File::options()
                .append(true)
                .open(DMARC_REPORT_LOG_FILEPATH)?;

            writeln!(f, "{result}")?;

            // self.aggregate().await;
        }

        Ok(())
    }

    pub async fn aggregate(&self) -> std::io::Result<()> {
        let mut output = vec![];
        let file = File::open(DMARC_REPORT_LOG_FILEPATH)?;
        let lines = BufReader::new(file).lines();

        for line in lines.map_while(Result::ok) {
            output.push(serde_json::from_str::<ErrorRecord>(&line)?);
        }

        let mut errors_grouped_by_email: HashMap<String, HashMap<IpAddr, Vec<ErrorRecord>>> =
            HashMap::new();

        'outer: for record in output {
            for (email, group) in errors_grouped_by_email.iter_mut() {
                if email == &record.email {
                    group
                        .entry(record.source_ip)
                        .and_modify(|x| x.push(record.clone()))
                        .or_insert_with(|| vec![record]);
                    continue 'outer;
                }
            }

            let mut new_group = HashMap::new();

            let record_email = record.email.clone();
            new_group.insert(record.source_ip.clone(), vec![record]);
            errors_grouped_by_email.insert(record_email, new_group);
        }

        for (email, errors_grouped_by_ip) in errors_grouped_by_email.iter_mut() {
            let mut errors = vec![];
            let mut record = vec![];

            //we know this is safe to do because for this list to be present, we will have found it earlier
            let (_, first_records) = errors_grouped_by_ip.iter().next().unwrap();

            let version = first_records[0].version.clone();
            let org_name = first_records[0].org_name.clone();
            let email = email.clone();
            let extra_contact_info = first_records[0].extra_contact_info.clone();

            let mut date_range = DateRange::new(first_records[0].when, first_records[0].when);

            let report_id = Uuid::new_v4().to_string();

            let domain = first_records[0].domain.clone();
            let align_dkim = first_records[0].align_dkim;
            let align_spf = first_records[0].align_spf;
            let policy = first_records[0].policy;
            let subdomain_policy = first_records[0].subdomain_policy;
            let rate = first_records[0].rate;
            let report_failure = first_records[0].report_failure;

            for (ip, error_group_for_ip) in errors_grouped_by_ip.iter_mut() {
                let row = Row {
                    source_ip: *ip,
                    count: error_group_for_ip.len() as u64,
                    policy_evaluated: error_group_for_ip[0].policy_evaluated.clone(),
                };

                let mut results = Results {
                    row,
                    identifiers: Identifier {
                        envelope_to: vec![],
                        envelope_from: vec![],
                        header_from: String::new(),
                    },
                    auth_results: AuthResults {
                        dkim: vec![],
                        spf: vec![],
                    },
                };

                for group_error in error_group_for_ip.iter() {
                    errors.push(group_error.error.clone());

                    if date_range.begin > group_error.when {
                        date_range.begin = group_error.when;
                    }

                    if date_range.end < group_error.when {
                        date_range.end = group_error.when;
                    }

                    results
                        .identifiers
                        .envelope_from
                        .extend_from_slice(&group_error.identifier.envelope_from);
                    results
                        .identifiers
                        .envelope_to
                        .extend_from_slice(&group_error.identifier.envelope_to);

                    results
                        .auth_results
                        .dkim
                        .extend_from_slice(&group_error.auth_results.dkim);
                    results
                        .auth_results
                        .spf
                        .extend_from_slice(&group_error.auth_results.spf);
                }

                record.push(results);
            }

            let _feedback = Feedback {
                version,
                metadata: ReportMetadata {
                    org_name,
                    email,
                    extra_contact_info,
                    report_id,
                    date_range,
                    error: errors,
                },
                policy: PolicyPublished::new(
                    domain,
                    align_dkim,
                    align_spf,
                    policy,
                    subdomain_policy,
                    rate,
                    report_failure,
                ),
                record,
            };

            // if let Ok(result) = instant_xml::to_string(&feedback) {
            //     println!("log: {}", result);
            // }
        }

        Ok(())
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

// The output is wrapped in a Result to allow matching on errors.
// Returns an Iterator to the Reader of the lines of the file.
fn read_lines<P>(filename: P) -> std::io::Result<std::io::Lines<std::io::BufReader<File>>>
where
    P: AsRef<std::path::Path>,
{
    let file = File::open(filename)?;
    Ok(std::io::BufReader::new(file).lines())
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
