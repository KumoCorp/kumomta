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
use bstr::BString;
use chrono::{DateTime, Utc};
use config::{declare_event, load_config};
use dns_resolver::Resolver;
use mailparsing::AuthenticationResult;
use parking_lot::FairMutex as Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::{self, Receiver, Sender};
use uuid::Uuid;

mod types;

#[cfg(test)]
mod tests;

const DMARC_REPORT_LOG_FILEPATH: &'static str = "/var/log/kumomta/dmarc.log";

declare_event! {
    static DMARC_REPORT_GENERATED: Single("dmarc_report_generated",
        report_content: String,
        report_email: String) -> ();
}

static RECORD_STREAM: LazyLock<Mutex<Option<mpsc::Sender<ErrorRecord>>>> =
    LazyLock::new(|| Mutex::new(None));

const DEFAULT_REPORT_WINDOW_SIZE: Duration = Duration::new(60 * 60, 0);
static REPORTER_CONFIG: LazyLock<Mutex<ReporterConfig>> = LazyLock::new(|| Default::default());

#[derive(Default)]
struct ReporterConfig {
    /// How long between sending reports.  You can pick a better name!
    window_size: Duration,
    /// Where to put the records on disk.  Default `/var/log/kumomta/dmarc-reports`
    log_dir: PathBuf,
}

fn get_record_stream() -> Option<Sender<ErrorRecord>> {
    RECORD_STREAM.lock().clone()
}

async fn send_generated_report(report_content: String, report_email: String) -> anyhow::Result<()> {
    let mut config = load_config().await?;

    config
        .async_call_callback(&DMARC_REPORT_GENERATED, (report_content, report_email))
        .await
}

pub fn startup_dmarc_reporter() {
    tokio::spawn(async move {
        // Do the initial flush of all pre-existing records that need to be reported from the previous instance
        match send_aggregated_reports().await {
            Err(_err) => {
                // todo: do something with this error
            }
            _ => {}
        }

        let (sender, receiver) = mpsc::channel::<ErrorRecord>(100);

        dmarc_reporter_loop(receiver).await;

        *RECORD_STREAM.lock() = Some(sender);
    });
}

pub(crate) async fn capture_error_record(
    record: ErrorRecord,
) -> Result<(), SendError<ErrorRecord>> {
    // Take a copy of the sending stream for ourselves so we can safely send on it
    let stream = get_record_stream();
    if let Some(stream) = stream {
        stream.send(record).await
    } else {
        Ok(())
    }
}

pub(crate) async fn send_aggregated_reports() -> anyhow::Result<()> {
    let email_reports = aggregate_errors().await;

    if let Ok(email_reports) = email_reports {
        for (email, report) in email_reports {
            if let Ok(result) = instant_xml::to_string(&report) {
                send_generated_report(result, email).await?;
            }
        }
    } else {
        // todo: we need a way to warn the user the report could not be processed
    }

    Ok(())
}

fn get_current_log_filename() -> String {
    let secs_since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);

    match secs_since_epoch {
        Ok(duration) => {
            let secs = duration.as_secs();
            let secs = secs - secs % (60 * 60);

            format!("{}.{}", DMARC_REPORT_LOG_FILEPATH, secs)
        }
        _ => DMARC_REPORT_LOG_FILEPATH.into(),
    }
}

async fn dmarc_reporter_loop<'a>(mut receiver: Receiver<ErrorRecord>) {
    // Start initial timeout as the remaining time before the end of the current interval

    let secs_since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);

    let window_in_seconds = DEFAULT_REPORT_WINDOW_SIZE.as_secs();

    let mut remaining_time = match secs_since_epoch {
        Ok(duration) => {
            let secs = duration.as_secs();

            Duration::from_secs(secs % window_in_seconds)
        }
        _ => DEFAULT_REPORT_WINDOW_SIZE,
    };

    loop {
        match tokio::time::timeout(remaining_time, receiver.recv()).await {
            Ok(r) => {
                match r {
                    Some(error_record) => {
                        let mut incoming_batch = vec![error_record];

                        while !receiver.is_empty() {
                            match receiver.recv().await {
                                Some(next_record) => {
                                    incoming_batch.push(next_record);
                                }
                                _ => {
                                    break;
                                }
                            }
                        }

                        let current_log = get_current_log_filename();

                        if let Ok(mut f) = File::options().append(true).open(current_log) {
                            for error_record in incoming_batch {
                                // TODO how do we want this to report out if we can't serialise for the report?
                                if let Ok(result) = serde_json::to_string(&error_record) {
                                    let _ = writeln!(f, "{result}");
                                }
                            }
                        }
                    }
                    None => {
                        // break?
                    }
                }
            }
            Err(_) => {
                // Timeout reached
                // todo: handle when reports fail to send
                let _ = send_aggregated_reports().await;

                remaining_time = match secs_since_epoch {
                    Ok(duration) => {
                        let secs = duration.as_secs();

                        Duration::from_secs(secs % window_in_seconds)
                    }
                    _ => DEFAULT_REPORT_WINDOW_SIZE,
                }
            }
        }
    }
}

pub async fn aggregate_errors() -> anyhow::Result<HashMap<String, Feedback>> {
    let mut input_records = vec![];

    let current_log = get_current_log_filename();

    let file_blob = format!("{}*", DMARC_REPORT_LOG_FILEPATH);
    let mut matching_historical_logs = vec![];

    if let Ok(paths) = glob::glob(&file_blob) {
        for file in paths {
            if let Ok(file) = file {
                if file != current_log {
                    matching_historical_logs.push(file);
                }
            }
        }
    }

    for matching_historical_log in matching_historical_logs {
        {
            let file = File::open(&matching_historical_log)?;
            let lines = BufReader::new(file).lines();

            for line in lines.map_while(Result::ok) {
                let result: anyhow::Result<ErrorRecord> =
                    serde_json::from_str::<ErrorRecord>(&line)
                        .map_err(|error| {
                            anyhow::Error::new(error).context(format!(
                                "Failed to decode a line from the DMARC report file \
           {}. \
           The line was: {line}. \
           Is the file corrupt?",
                                matching_historical_log.to_string_lossy()
                            ))
                        })
                        .into();

                input_records.push(result?);
            }
        }

        // What should we do if this fails to remove?
        let _ = std::fs::remove_file(matching_historical_log);
    }

    let mut errors_grouped_by_email: HashMap<String, BTreeMap<IpAddr, Vec<ErrorRecord>>> =
        HashMap::new();

    for record in input_records {
        let entry = errors_grouped_by_email.entry(record.email.clone());
        let record_source_ip = record.source_ip.clone();

        entry
            .and_modify(|entry| {
                entry
                    .entry(record.source_ip)
                    .and_modify(|x| x.push(record.clone()))
                    .or_insert_with(|| vec![record.clone()]);
            })
            .or_insert({
                let mut new_group = BTreeMap::new();

                new_group.insert(record_source_ip, vec![record]);

                new_group
            });
    }

    let mut feedback_for_emails = HashMap::new();

    for (email, errors_grouped_by_ip) in errors_grouped_by_email.iter_mut() {
        let mut errors = vec![];
        let mut record = vec![];

        //we know this is safe to do because for this list to be present, we will have found it earlier
        let (_, first_records) = errors_grouped_by_ip
            .iter()
            .next()
            .expect("guaranteed to not be empty by the logic above");

        let first_record = &first_records[0];

        let version = first_record.version.clone();
        let org_name = first_record.org_name.clone();
        let email = email.clone();
        let extra_contact_info = first_record.extra_contact_info.clone();

        let mut date_range = DateRange::new(first_record.when, first_record.when);

        let report_id = Uuid::new_v4().to_string();

        let domain = first_record.domain.clone();
        let align_dkim = first_record.align_dkim;
        let align_spf = first_record.align_spf;
        let policy = first_record.policy;
        let subdomain_policy = first_record.subdomain_policy;
        let rate = first_record.rate;
        let report_failure = first_record.report_failure;

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

                date_range.begin = std::cmp::min(date_range.begin, group_error.when);
                date_range.end = std::cmp::max(date_range.end, group_error.when);

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

        let feedback = Feedback {
            version,
            metadata: ReportMetadata {
                org_name,
                email: email.clone(),
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

        feedback_for_emails.insert(email, feedback);
    }

    Ok(feedback_for_emails)
}

pub struct DmarcPassContext {
    /// Domain of the sender in the "From:"
    pub from_domain: String,

    /// Domain that provides the sought-after authorization information.
    ///
    /// The "MAIL FROM" email address if available.
    pub mail_from_domain: Option<String>,

    /// The envelope to
    pub recipient_domain_list: Vec<String>,

    /// The source IP address
    pub received_from: String,

    /// The results of the DKIM part of the checks
    pub dkim_results: Vec<AuthenticationResult>,

    /// The results of the SPF part of the checks
    pub spf_result: AuthenticationResult,

    /// The additional information needed to perform reporting
    pub reporting_info: Option<ReportingInfo>,
}

impl DmarcPassContext {
    pub async fn check(self, resolver: &dyn Resolver) -> DispositionWithContext {
        let Self {
            from_domain,
            mail_from_domain,
            recipient_domain_list: recipient_list,
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
    ) -> anyhow::Result<()> {
        let source_ip = self.received_from.parse()?;

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

            capture_error_record(error_record).await?;
        }

        Ok(())
    }

    pub async fn check(&mut self, resolver: &dyn Resolver) -> DispositionWithContext {
        let dmarc_domain = format!("_dmarc.{}", self.from_domain);
        match fetch_dmarc_records(&dmarc_domain, resolver).await {
            DmarcRecordResolution::Records(records) => {
                for record in records {
                    let mut result = record
                        .evaluate(self, &dmarc_domain, SenderDomainAlignment::Exact)
                        .await;
                    result.props.extend(policy_tags(&record));
                    return result;
                }
            }
            x => {
                let normalized_from = psl_utils::normalize_domain(self.from_domain);
                if let Some(organizational_domain) = psl_utils::domain_str(&normalized_from) {
                    if organizational_domain != normalized_from {
                        let address = format!("_dmarc.{}", organizational_domain);
                        match fetch_dmarc_records(&address, resolver).await {
                            DmarcRecordResolution::TempError => {
                                return DispositionWithContext {
                                    result: Disposition::TempError,
                                    context: format!(
                                        "DNS records could not be resolved for {}",
                                        address
                                    ),
                                    props: BTreeMap::new(),
                                }
                            }
                            DmarcRecordResolution::PermError => {
                                return DispositionWithContext {
                                    result: Disposition::PermError,
                                    context: format!("no DMARC records found for {}", address),
                                    props: BTreeMap::new(),
                                }
                            }
                            DmarcRecordResolution::Records(records) => {
                                for record in records {
                                    let mut result = record
                                        .evaluate(
                                            self,
                                            &address,
                                            SenderDomainAlignment::OrganizationalDomain,
                                        )
                                        .await;
                                    result.props.extend(policy_tags(&record));
                                    return result;
                                }
                            }
                        }
                    } else {
                        return DispositionWithContext {
                            result: x.into(),
                            context: format!("no DMARC records found for {}", &self.from_domain),
                            props: BTreeMap::new(),
                        };
                    }
                }
            }
        }

        DispositionWithContext {
            result: Disposition::None,
            context: format!("no DMARC records found for {}", &self.from_domain),
            props: BTreeMap::new(),
        }
    }
}

fn policy_tags(record: &Record) -> BTreeMap<String, BString> {
    let mut props = BTreeMap::new();

    for (tag, value) in record.tags() {
        props.insert(format!("policy.{tag}").into(), value.as_str().into());
    }

    props
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
