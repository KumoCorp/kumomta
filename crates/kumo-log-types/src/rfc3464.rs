//! This module parses out RFC3464 delivery status reports
//! from an email message
use crate::rfc5965::{
    extract_headers, extract_single, extract_single_conv, extract_single_req, DateTimeRfc2822,
};
use crate::{JsonLogRecord, RecordType};
use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use mailparsing::MimePart;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize, Copy, Clone, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReportAction {
    Failed,
    Delayed,
    Delivered,
    Relayed,
    Expanded,
}

impl std::fmt::Display for ReportAction {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let label = match self {
            Self::Failed => "failed",
            Self::Delayed => "delayed",
            Self::Delivered => "delivered",
            Self::Relayed => "relayed",
            Self::Expanded => "expanded",
        };
        write!(fmt, "{label}")
    }
}

impl FromStr for ReportAction {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> anyhow::Result<Self> {
        Ok(match input {
            "failed" => Self::Failed,
            "delayed" => Self::Delayed,
            "delivered" => Self::Delivered,
            "relayed" => Self::Relayed,
            "expanded" => Self::Expanded,
            _ => anyhow::bail!("invalid action type {input}"),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct ReportStatus {
    pub class: u8,
    pub subject: u16,
    pub detail: u16,
    pub comment: Option<String>,
}

impl std::fmt::Display for ReportStatus {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}.{}.{}", self.class, self.subject, self.detail)?;
        if let Some(comment) = &self.comment {
            write!(fmt, " {comment}")?;
        }
        Ok(())
    }
}

impl From<&rfc5321::Response> for ReportStatus {
    fn from(response: &rfc5321::Response) -> ReportStatus {
        let (class, subject, detail) = match &response.enhanced_code {
            Some(enh) => (enh.class, enh.subject, enh.detail),
            None => {
                if response.code >= 500 {
                    (5, 0, 0)
                } else if response.code >= 400 {
                    (4, 0, 0)
                } else if response.code >= 200 && response.code < 300 {
                    (2, 0, 0)
                } else {
                    (4, 0, 0)
                }
            }
        };
        ReportStatus {
            class,
            subject,
            detail,
            comment: Some(response.content.clone()),
        }
    }
}

impl FromStr for ReportStatus {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> anyhow::Result<Self> {
        let mut parts: Vec<_> = input.split(' ').collect();

        let mut status = parts[0].split('.');
        let class = status
            .next()
            .ok_or_else(|| anyhow!("invalid Status: {input}"))?
            .parse()
            .context("parsing status.class")?;
        let subject = status
            .next()
            .ok_or_else(|| anyhow!("invalid Status: {input}"))?
            .parse()
            .context("parsing status.subject")?;
        let detail = status
            .next()
            .ok_or_else(|| anyhow!("invalid Status: {input}"))?
            .parse()
            .context("parsing status.detail")?;

        parts.remove(0);
        let comment = if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        };

        Ok(Self {
            class,
            subject,
            detail,
            comment,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct RemoteMta {
    pub mta_type: String,
    pub name: String,
}

impl std::fmt::Display for RemoteMta {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}; {}", self.mta_type, self.name)
    }
}

impl FromStr for RemoteMta {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> anyhow::Result<Self> {
        let (mta_type, name) = input
            .split_once(";")
            .ok_or_else(|| anyhow!("expected 'name-type; name', got {input}"))?;
        Ok(Self {
            mta_type: mta_type.trim().to_string(),
            name: name.trim().to_string(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct Recipient {
    pub recipient_type: String,
    pub recipient: String,
}

impl std::fmt::Display for Recipient {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{};{}", self.recipient_type, self.recipient)
    }
}

impl FromStr for Recipient {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> anyhow::Result<Self> {
        let (recipient_type, recipient) = input
            .split_once(";")
            .ok_or_else(|| anyhow!("expected 'recipient-type; recipient', got {input}"))?;
        Ok(Self {
            recipient_type: recipient_type.trim().to_string(),
            recipient: recipient.trim().to_string(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct DiagnosticCode {
    pub diagnostic_type: String,
    pub diagnostic: String,
}

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}; {}", self.diagnostic_type, self.diagnostic)
    }
}

impl FromStr for DiagnosticCode {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> anyhow::Result<Self> {
        let (diagnostic_type, diagnostic) = input
            .split_once(";")
            .ok_or_else(|| anyhow!("expected 'diagnostic-type; diagnostic', got {input}"))?;
        Ok(Self {
            diagnostic_type: diagnostic_type.trim().to_string(),
            diagnostic: diagnostic.trim().to_string(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct PerRecipientReportEntry {
    pub final_recipient: Recipient,
    pub action: ReportAction,
    pub status: ReportStatus,
    pub original_recipient: Option<Recipient>,
    pub remote_mta: Option<RemoteMta>,
    pub diagnostic_code: Option<DiagnosticCode>,
    pub last_attempt_date: Option<DateTime<Utc>>,
    pub final_log_id: Option<String>,
    pub will_retry_until: Option<DateTime<Utc>>,
    pub extensions: BTreeMap<String, Vec<String>>,
}

impl std::fmt::Display for PerRecipientReportEntry {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        if let Some(orig) = &self.original_recipient {
            write!(fmt, "Original-Recipient: {orig}\r\n")?;
        }
        write!(fmt, "Final-Recipient: {}\r\n", self.final_recipient)?;
        write!(fmt, "Action: {}\r\n", self.action)?;
        write!(fmt, "Status: {}\r\n", self.status)?;
        if let Some(mta) = &self.remote_mta {
            write!(fmt, "Remote-MTA: {mta}\r\n")?;
        }
        if let Some(code) = &self.diagnostic_code {
            write!(fmt, "Diagnostic-Code: {code}\r\n")?;
        }
        if let Some(when) = &self.last_attempt_date {
            write!(fmt, "Last-Attempt-Date: {}\r\n", when.to_rfc2822())?;
        }
        if let Some(id) = &self.final_log_id {
            write!(fmt, "Final-Log-Id: {id}\r\n")?;
        }
        if let Some(when) = &self.will_retry_until {
            write!(fmt, "Will-Retry-Until: {}\r\n", when.to_rfc2822())?;
        }
        for (k, vlist) in &self.extensions {
            for v in vlist {
                write!(fmt, "{k}: {v}\r\n")?;
            }
        }
        Ok(())
    }
}

impl PerRecipientReportEntry {
    fn parse(part: &str) -> anyhow::Result<Self> {
        let mut extensions = extract_headers(part.as_bytes())?;

        let original_recipient = extract_single("original-recipient", &mut extensions)?;
        let final_recipient = extract_single_req("final-recipient", &mut extensions)?;
        let remote_mta = extract_single("remote-mta", &mut extensions)?;

        let last_attempt_date = extract_single_conv::<DateTimeRfc2822, DateTime<Utc>>(
            "last-attempt-date",
            &mut extensions,
        )?;
        let will_retry_until = extract_single_conv::<DateTimeRfc2822, DateTime<Utc>>(
            "will-retry-until",
            &mut extensions,
        )?;
        let final_log_id = extract_single("final-log-id", &mut extensions)?;

        let action = extract_single_req("action", &mut extensions)?;
        let status = extract_single_req("status", &mut extensions)?;
        let diagnostic_code = extract_single("diagnostic-code", &mut extensions)?;

        Ok(Self {
            final_recipient,
            action,
            status,
            diagnostic_code,
            original_recipient,
            remote_mta,
            last_attempt_date,
            final_log_id,
            will_retry_until,
            extensions,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct PerMessageReportEntry {
    pub original_envelope_id: Option<String>,
    pub reporting_mta: RemoteMta,
    pub dsn_gateway: Option<RemoteMta>,
    pub received_from_mta: Option<RemoteMta>,
    pub arrival_date: Option<DateTime<Utc>>,
    pub extensions: BTreeMap<String, Vec<String>>,
}

impl std::fmt::Display for PerMessageReportEntry {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "Reporting-MTA: {}\r\n", self.reporting_mta)?;
        if let Some(id) = &self.original_envelope_id {
            write!(fmt, "Original-Envelope-Id: {id}\r\n")?;
        }
        if let Some(dsn_gateway) = &self.dsn_gateway {
            write!(fmt, "DSN-Gateway: {dsn_gateway}\r\n")?;
        }
        if let Some(mta) = &self.received_from_mta {
            write!(fmt, "Received-From-MTA: {mta}\r\n")?;
        }
        if let Some(when) = &self.arrival_date {
            write!(fmt, "Arrival-Date: {}\r\n", when.to_rfc2822())?;
        }
        for (k, vlist) in &self.extensions {
            for v in vlist {
                write!(fmt, "{k}: {v}\r\n")?;
            }
        }

        Ok(())
    }
}

impl PerMessageReportEntry {
    fn parse(part: &str) -> anyhow::Result<Self> {
        let mut extensions = extract_headers(part.as_bytes())?;

        let reporting_mta = extract_single_req("reporting-mta", &mut extensions)?;
        let original_envelope_id = extract_single("original-envelope-id", &mut extensions)?;
        let dsn_gateway = extract_single("dsn-gateway", &mut extensions)?;
        let received_from_mta = extract_single("received-from-mta", &mut extensions)?;

        let arrival_date =
            extract_single_conv::<DateTimeRfc2822, DateTime<Utc>>("arrival-date", &mut extensions)?;

        Ok(Self {
            original_envelope_id,
            reporting_mta,
            dsn_gateway,
            received_from_mta,
            arrival_date,
            extensions,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct Report {
    pub per_message: PerMessageReportEntry,
    pub per_recipient: Vec<PerRecipientReportEntry>,
    pub original_message: Option<String>,
}

pub(crate) fn content_type(part: &MimePart) -> Option<String> {
    let ct = part.headers().content_type().ok()??;
    Some(ct.value)
}

impl Report {
    pub fn parse(input: &[u8]) -> anyhow::Result<Option<Self>> {
        let mail = MimePart::parse(input).with_context(|| {
            format!(
                "Report::parse top; input is {:?}",
                String::from_utf8_lossy(input)
            )
        })?;

        if content_type(&mail).as_deref() != Some("multipart/report") {
            return Ok(None);
        }

        let mut original_message = None;

        for part in mail.child_parts() {
            let ct = content_type(part);
            let ct = ct.as_deref();
            if ct == Some("message/rfc822") || ct == Some("text/rfc822-headers") {
                original_message = Some(part.raw_body().replace("\r\n", "\n"));
            }
        }

        for part in mail.child_parts() {
            let ct = content_type(part);
            let ct = ct.as_deref();
            if ct == Some("message/delivery-status") || ct == Some("message/global-delivery-status")
            {
                return Ok(Some(Self::parse_inner(part, original_message)?));
            }
        }

        anyhow::bail!("delivery-status part missing");
    }

    fn parse_inner(part: &MimePart, original_message: Option<String>) -> anyhow::Result<Self> {
        let body = part.body()?.to_string_lossy().replace("\r\n", "\n");
        let mut parts = body.trim().split("\n\n");

        let per_message = parts
            .next()
            .ok_or_else(|| anyhow!("missing per-message section"))?;
        let per_message = PerMessageReportEntry::parse(per_message)?;
        let mut per_recipient = vec![];
        for part in parts {
            let part = PerRecipientReportEntry::parse(part)?;
            per_recipient.push(part);
        }

        Ok(Self {
            per_message,
            per_recipient,
            original_message,
        })
    }

    /// msg: the message that experienced an issue
    /// log: the corresponding log record from the issue
    pub fn generate(
        params: &ReportGenerationParams,
        msg: Option<&MimePart<'_>>,
        log: &JsonLogRecord,
    ) -> anyhow::Result<Option<MimePart<'static>>> {
        let action = match &log.kind {
            RecordType::Bounce
                if params.enable_bounce && log.delivery_protocol.as_deref() == Some("ESMTP") =>
            {
                ReportAction::Failed
            }
            RecordType::Expiration if params.enable_expiration => ReportAction::Failed,
            _ => return Ok(None),
        };

        let arrival_date = Some(log.created);

        let per_message = PerMessageReportEntry {
            arrival_date,
            dsn_gateway: None,
            extensions: Default::default(),
            original_envelope_id: None,
            received_from_mta: None,
            reporting_mta: params.reporting_mta.clone(),
        };
        let per_recipient = PerRecipientReportEntry {
            action,
            extensions: Default::default(),
            status: (&log.response).into(),
            diagnostic_code: Some(DiagnosticCode {
                diagnostic_type: "smtp".into(),
                diagnostic: log.response.to_single_line(),
            }),
            final_log_id: None,
            original_recipient: None,
            final_recipient: Recipient {
                recipient_type: "rfc822".to_string(),
                recipient: log.recipient.to_string(),
            },
            remote_mta: log.peer_address.as_ref().map(|addr| RemoteMta {
                mta_type: "dns".to_string(),
                name: addr.name.to_string(),
            }),
            last_attempt_date: Some(log.timestamp),
            will_retry_until: None,
        };

        let mut parts = vec![];

        let exposition = match &log.kind {
            RecordType::Bounce => {
                let mut data = format!(
                    "The message was received at {created}\r\n\
                    from {sender} and addressed to {recipient}.\r\n\
                    ",
                    created = log.created.to_rfc2822(),
                    sender = log.sender,
                    recipient = log.recipient
                );
                if let Some(peer) = &log.peer_address {
                    data.push_str(&format!(
                        "While communicating with {host} ({ip}):\r\n\
                        Response: {resp}\r\n",
                        host = peer.name,
                        ip = peer.addr,
                        resp = log.response.to_single_line(),
                    ));
                } else {
                    data.push_str(&format!("Status: {}\r\n", log.response.to_single_line()));
                }

                data.push_str(
                    "\r\nThe message will be deleted from the queue.\r\n\
                    No further attempts will be made to deliver it.\r\n",
                );

                data
            }
            RecordType::Expiration => {
                format!(
                    "The message was received at {created}\r\n\
                    from {sender} and addressed to {recipient}.\r\n\
                    Status: {status}\r\n\
                    The message will be deleted from the queue.\r\n\
                    No further attempts will be made to deliver it.\r\n\
                    ",
                    created = log.created.to_rfc2822(),
                    sender = log.sender,
                    recipient = log.recipient,
                    status = log.response.to_single_line()
                )
            }
            _ => unreachable!(),
        };

        parts.push(MimePart::new_text_plain(&exposition)?);

        let status_text = format!("{per_message}\r\n{per_recipient}\r\n");
        parts.push(MimePart::new_text("message/delivery-status", &status_text)?);

        match (params.include_original_message, msg) {
            (IncludeOriginalMessage::No, _) | (_, None) => {}
            (IncludeOriginalMessage::HeadersOnly, Some(msg)) => {
                let mut data = vec![];
                for hdr in msg.headers().iter() {
                    hdr.write_header(&mut data).ok();
                }
                parts.push(MimePart::new_no_transfer_encoding(
                    "text/rfc822-headers",
                    &data,
                )?);
            }
            (IncludeOriginalMessage::FullContent, Some(msg)) => {
                let mut data = vec![];
                msg.write_message(&mut data).ok();
                parts.push(MimePart::new_no_transfer_encoding("message/rfc822", &data)?);
            }
        };

        let mut report_msg = MimePart::new_multipart(
            "multipart/report",
            parts,
            if params.stable_content {
                Some("report-boundary")
            } else {
                None
            },
        )?;

        let mut ct = report_msg
            .headers()
            .content_type()?
            .expect("assigned during construction");
        ct.set("report-type", "delivery-status");
        report_msg.headers_mut().set_content_type(ct)?;
        report_msg.headers_mut().set_subject("Returned mail")?;
        report_msg.headers_mut().set_mime_version("1.0")?;

        let message_id = if params.stable_content {
            format!("<UUID@{}>", params.reporting_mta.name)
        } else {
            let id = uuid_helper::now_v1();
            format!("<{id}@{}>", params.reporting_mta.name)
        };
        report_msg
            .headers_mut()
            .set_message_id(message_id.as_str())?;
        report_msg.headers_mut().set_to(log.sender.as_str())?;

        let from = format!(
            "Mail Delivery Subsystem <mailer-daemon@{}>",
            params.reporting_mta.name
        );
        report_msg.headers_mut().set_from(from.as_str())?;

        Ok(Some(report_msg))
    }
}

#[derive(Default, Debug, PartialEq, Clone, Copy, Deserialize)]
pub enum IncludeOriginalMessage {
    #[default]
    No,
    HeadersOnly,
    FullContent,
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportGenerationParams {
    pub include_original_message: IncludeOriginalMessage,
    #[serde(default)]
    pub enable_expiration: bool,
    #[serde(default)]
    pub enable_bounce: bool,
    // If we decide to allow generating for delays in the future,
    // we'll probably add `enable_delay` here, but we'll also need
    // to have some kind of discriminating logic to decide when
    // to emit a DSN; probably should be a list of num_attempts
    // on which to emit? This is too fiddly to design for right
    // now, considering that none of our target userbase will
    // emit DSNs for delayed mail.
    pub reporting_mta: RemoteMta,

    /// When used for testing, use a stable mime boundary
    #[serde(default)]
    pub stable_content: bool,
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ResolvedAddress;
    use rfc5321::{EnhancedStatusCode, Response};

    fn make_message() -> MimePart<'static> {
        let mut part = MimePart::new_text_plain("hello there").unwrap();
        part.headers_mut().set_subject("Hello!").unwrap();

        part
    }

    fn make_bounce() -> JsonLogRecord {
        let nodeid = uuid_helper::now_v1();
        let created =
            chrono::DateTime::parse_from_rfc2822("Tue, 1 Jul 2003 10:52:37 +0200").unwrap();
        let now = chrono::DateTime::parse_from_rfc2822("Tue, 1 Jul 2003 12:52:37 +0200").unwrap();
        JsonLogRecord {
            kind: RecordType::Bounce,
            id: "ID".to_string(),
            nodeid,
            created: created.into(),
            bounce_classification: Default::default(),
            delivery_protocol: Some("ESMTP".to_string()),
            egress_pool: None,
            egress_source: None,
            feedback_report: None,
            headers: Default::default(),
            meta: Default::default(),
            num_attempts: 1,
            peer_address: Some(ResolvedAddress {
                name: "target.example.com".to_string(),
                addr: "42.42.42.42".to_string().try_into().unwrap(),
            }),
            provider_name: None,
            queue: "target.example.com".to_string(),
            reception_protocol: None,
            recipient: "recip@target.example.com".to_string(),
            sender: "sender@sender.example.com".to_string(),
            session_id: None,
            response: Response {
                code: 550,
                command: None,
                content: "no thanks".to_string(),
                enhanced_code: Some(EnhancedStatusCode {
                    class: 5,
                    subject: 7,
                    detail: 1,
                }),
            },
            site: "some-site".to_string(),
            size: 0,
            source_address: None,
            timestamp: now.into(),
            tls_cipher: None,
            tls_peer_subject_name: None,
            tls_protocol_version: None,
        }
    }

    fn make_expiration() -> JsonLogRecord {
        let nodeid = uuid_helper::now_v1();
        let created =
            chrono::DateTime::parse_from_rfc2822("Tue, 1 Jul 2003 10:52:37 +0200").unwrap();
        let now = chrono::DateTime::parse_from_rfc2822("Tue, 1 Jul 2003 12:52:37 +0200").unwrap();
        JsonLogRecord {
            kind: RecordType::Expiration,
            id: "ID".to_string(),
            nodeid,
            created: created.into(),
            bounce_classification: Default::default(),
            delivery_protocol: None,
            egress_pool: None,
            egress_source: None,
            feedback_report: None,
            headers: Default::default(),
            meta: Default::default(),
            num_attempts: 3,
            peer_address: None,
            provider_name: None,
            queue: "target.example.com".to_string(),
            reception_protocol: None,
            recipient: "recip@target.example.com".to_string(),
            sender: "sender@sender.example.com".to_string(),
            session_id: None,
            response: Response {
                code: 551,
                command: None,
                content: "Next delivery time would be at SOME TIME which exceeds the expiry time EXPIRES configured via set_scheduling".to_string(),
                enhanced_code: Some(EnhancedStatusCode {
                    class: 5,
                    subject: 4,
                    detail: 7,
                }),
            },
            site: "".to_string(),
            size: 0,
            source_address: None,
            timestamp: now.into(),
            tls_cipher: None,
            tls_peer_subject_name: None,
            tls_protocol_version: None,
        }
    }

    #[test]
    fn generate_expiration_with_headers() {
        let params = ReportGenerationParams {
            reporting_mta: RemoteMta {
                mta_type: "dns".to_string(),
                name: "mta1.example.com".to_string(),
            },
            enable_bounce: false,
            enable_expiration: true,
            include_original_message: IncludeOriginalMessage::HeadersOnly,
            stable_content: true,
        };

        let original_msg = make_message();

        let log = make_expiration();

        let report_msg = Report::generate(&params, Some(&original_msg), &log)
            .unwrap()
            .unwrap();
        let report_eml = report_msg.to_message_string();
        k9::snapshot!(
            &report_eml,
            r#"
Content-Type: multipart/report;\r
\tboundary="report-boundary";\r
\treport-type="delivery-status"\r
Subject: Returned mail\r
Mime-Version: 1.0\r
Message-ID: <UUID@mta1.example.com>\r
To: sender@sender.example.com\r
From: Mail Delivery Subsystem <mailer-daemon@mta1.example.com>\r
\r
--report-boundary\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Content-Transfer-Encoding: quoted-printable\r
\r
The message was received at Tue, 1 Jul 2003 08:52:37 +0000\r
from sender@sender.example.com and addressed to recip@target.example.com.\r
Status: 551 5.4.7 Next delivery time would be at SOME TIME which exceeds th=\r
e expiry time EXPIRES configured via set_scheduling\r
The message will be deleted from the queue.\r
No further attempts will be made to deliver it.\r
--report-boundary\r
Content-Type: message/delivery-status;\r
\tcharset="us-ascii"\r
Content-Transfer-Encoding: quoted-printable\r
\r
Reporting-MTA: dns; mta1.example.com\r
Arrival-Date: Tue, 1 Jul 2003 08:52:37 +0000\r
\r
Final-Recipient: rfc822;recip@target.example.com\r
Action: failed\r
Status: 5.4.7 Next delivery time would be at SOME TIME which exceeds the ex=\r
piry time EXPIRES configured via set_scheduling\r
Diagnostic-Code: smtp; 551 5.4.7 Next delivery time would be at SOME TIME w=\r
hich exceeds the expiry time EXPIRES configured via set_scheduling\r
Last-Attempt-Date: Tue, 1 Jul 2003 10:52:37 +0000\r
\r
--report-boundary\r
Content-Type: text/rfc822-headers\r
\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Subject: Hello!\r
--report-boundary--\r

"#
        );

        let round_trip = Report::parse(report_eml.as_bytes()).unwrap().unwrap();
        k9::snapshot!(
            &round_trip,
            r#"
Report {
    per_message: PerMessageReportEntry {
        original_envelope_id: None,
        reporting_mta: RemoteMta {
            mta_type: "dns",
            name: "mta1.example.com",
        },
        dsn_gateway: None,
        received_from_mta: None,
        arrival_date: Some(
            2003-07-01T08:52:37Z,
        ),
        extensions: {},
    },
    per_recipient: [
        PerRecipientReportEntry {
            final_recipient: Recipient {
                recipient_type: "rfc822",
                recipient: "recip@target.example.com",
            },
            action: Failed,
            status: ReportStatus {
                class: 5,
                subject: 4,
                detail: 7,
                comment: Some(
                    "Next delivery time would be at SOME TIME which exceeds the expiry time EXPIRES configured via set_scheduling",
                ),
            },
            original_recipient: None,
            remote_mta: None,
            diagnostic_code: Some(
                DiagnosticCode {
                    diagnostic_type: "smtp",
                    diagnostic: "551 5.4.7 Next delivery time would be at SOME TIME which exceeds the expiry time EXPIRES configured via set_scheduling",
                },
            ),
            last_attempt_date: Some(
                2003-07-01T10:52:37Z,
            ),
            final_log_id: None,
            will_retry_until: None,
            extensions: {},
        },
    ],
    original_message: Some(
        "Content-Type: text/plain;
\tcharset="us-ascii"
Subject: Hello!
",
    ),
}
"#
        );
    }

    #[test]
    fn generate_bounce_with_headers() {
        let params = ReportGenerationParams {
            reporting_mta: RemoteMta {
                mta_type: "dns".to_string(),
                name: "mta1.example.com".to_string(),
            },
            enable_bounce: true,
            enable_expiration: true,
            include_original_message: IncludeOriginalMessage::HeadersOnly,
            stable_content: true,
        };

        let original_msg = make_message();

        let log = make_bounce();

        let report_msg = Report::generate(&params, Some(&original_msg), &log)
            .unwrap()
            .unwrap();
        let report_eml = report_msg.to_message_string();
        k9::snapshot!(
            &report_eml,
            r#"
Content-Type: multipart/report;\r
\tboundary="report-boundary";\r
\treport-type="delivery-status"\r
Subject: Returned mail\r
Mime-Version: 1.0\r
Message-ID: <UUID@mta1.example.com>\r
To: sender@sender.example.com\r
From: Mail Delivery Subsystem <mailer-daemon@mta1.example.com>\r
\r
--report-boundary\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
\r
The message was received at Tue, 1 Jul 2003 08:52:37 +0000\r
from sender@sender.example.com and addressed to recip@target.example.com.\r
While communicating with target.example.com (42.42.42.42):\r
Response: 550 5.7.1 no thanks\r
\r
The message will be deleted from the queue.\r
No further attempts will be made to deliver it.\r
--report-boundary\r
Content-Type: message/delivery-status;\r
\tcharset="us-ascii"\r
\r
Reporting-MTA: dns; mta1.example.com\r
Arrival-Date: Tue, 1 Jul 2003 08:52:37 +0000\r
\r
Final-Recipient: rfc822;recip@target.example.com\r
Action: failed\r
Status: 5.7.1 no thanks\r
Remote-MTA: dns; target.example.com\r
Diagnostic-Code: smtp; 550 5.7.1 no thanks\r
Last-Attempt-Date: Tue, 1 Jul 2003 10:52:37 +0000\r
\r
--report-boundary\r
Content-Type: text/rfc822-headers\r
\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Subject: Hello!\r
--report-boundary--\r

"#
        );

        let round_trip = Report::parse(report_eml.as_bytes()).unwrap().unwrap();
        k9::snapshot!(
            &round_trip,
            r#"
Report {
    per_message: PerMessageReportEntry {
        original_envelope_id: None,
        reporting_mta: RemoteMta {
            mta_type: "dns",
            name: "mta1.example.com",
        },
        dsn_gateway: None,
        received_from_mta: None,
        arrival_date: Some(
            2003-07-01T08:52:37Z,
        ),
        extensions: {},
    },
    per_recipient: [
        PerRecipientReportEntry {
            final_recipient: Recipient {
                recipient_type: "rfc822",
                recipient: "recip@target.example.com",
            },
            action: Failed,
            status: ReportStatus {
                class: 5,
                subject: 7,
                detail: 1,
                comment: Some(
                    "no thanks",
                ),
            },
            original_recipient: None,
            remote_mta: Some(
                RemoteMta {
                    mta_type: "dns",
                    name: "target.example.com",
                },
            ),
            diagnostic_code: Some(
                DiagnosticCode {
                    diagnostic_type: "smtp",
                    diagnostic: "550 5.7.1 no thanks",
                },
            ),
            last_attempt_date: Some(
                2003-07-01T10:52:37Z,
            ),
            final_log_id: None,
            will_retry_until: None,
            extensions: {},
        },
    ],
    original_message: Some(
        "Content-Type: text/plain;
\tcharset="us-ascii"
Subject: Hello!
",
    ),
}
"#
        );
    }
    #[test]
    fn generate_bounce_with_message() {
        let params = ReportGenerationParams {
            reporting_mta: RemoteMta {
                mta_type: "dns".to_string(),
                name: "mta1.example.com".to_string(),
            },
            enable_bounce: true,
            enable_expiration: true,
            include_original_message: IncludeOriginalMessage::FullContent,
            stable_content: true,
        };

        let original_msg = make_message();

        let log = make_bounce();

        let report_msg = Report::generate(&params, Some(&original_msg), &log)
            .unwrap()
            .unwrap();
        let report_eml = report_msg.to_message_string();
        k9::snapshot!(
            &report_eml,
            r#"
Content-Type: multipart/report;\r
\tboundary="report-boundary";\r
\treport-type="delivery-status"\r
Subject: Returned mail\r
Mime-Version: 1.0\r
Message-ID: <UUID@mta1.example.com>\r
To: sender@sender.example.com\r
From: Mail Delivery Subsystem <mailer-daemon@mta1.example.com>\r
\r
--report-boundary\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
\r
The message was received at Tue, 1 Jul 2003 08:52:37 +0000\r
from sender@sender.example.com and addressed to recip@target.example.com.\r
While communicating with target.example.com (42.42.42.42):\r
Response: 550 5.7.1 no thanks\r
\r
The message will be deleted from the queue.\r
No further attempts will be made to deliver it.\r
--report-boundary\r
Content-Type: message/delivery-status;\r
\tcharset="us-ascii"\r
\r
Reporting-MTA: dns; mta1.example.com\r
Arrival-Date: Tue, 1 Jul 2003 08:52:37 +0000\r
\r
Final-Recipient: rfc822;recip@target.example.com\r
Action: failed\r
Status: 5.7.1 no thanks\r
Remote-MTA: dns; target.example.com\r
Diagnostic-Code: smtp; 550 5.7.1 no thanks\r
Last-Attempt-Date: Tue, 1 Jul 2003 10:52:37 +0000\r
\r
--report-boundary\r
Content-Type: message/rfc822\r
\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Subject: Hello!\r
\r
hello there\r
--report-boundary--\r

"#
        );

        let round_trip = Report::parse(report_eml.as_bytes()).unwrap().unwrap();
        k9::snapshot!(
            &round_trip,
            r#"
Report {
    per_message: PerMessageReportEntry {
        original_envelope_id: None,
        reporting_mta: RemoteMta {
            mta_type: "dns",
            name: "mta1.example.com",
        },
        dsn_gateway: None,
        received_from_mta: None,
        arrival_date: Some(
            2003-07-01T08:52:37Z,
        ),
        extensions: {},
    },
    per_recipient: [
        PerRecipientReportEntry {
            final_recipient: Recipient {
                recipient_type: "rfc822",
                recipient: "recip@target.example.com",
            },
            action: Failed,
            status: ReportStatus {
                class: 5,
                subject: 7,
                detail: 1,
                comment: Some(
                    "no thanks",
                ),
            },
            original_recipient: None,
            remote_mta: Some(
                RemoteMta {
                    mta_type: "dns",
                    name: "target.example.com",
                },
            ),
            diagnostic_code: Some(
                DiagnosticCode {
                    diagnostic_type: "smtp",
                    diagnostic: "550 5.7.1 no thanks",
                },
            ),
            last_attempt_date: Some(
                2003-07-01T10:52:37Z,
            ),
            final_log_id: None,
            will_retry_until: None,
            extensions: {},
        },
    ],
    original_message: Some(
        "Content-Type: text/plain;
\tcharset="us-ascii"
Subject: Hello!

hello there
",
    ),
}
"#
        );
    }

    #[test]
    fn generate_bounce_no_message() {
        let params = ReportGenerationParams {
            reporting_mta: RemoteMta {
                mta_type: "dns".to_string(),
                name: "mta1.example.com".to_string(),
            },
            enable_bounce: true,
            enable_expiration: true,
            include_original_message: IncludeOriginalMessage::No,
            stable_content: true,
        };

        let original_msg = make_message();

        let log = make_bounce();

        let report_msg = Report::generate(&params, Some(&original_msg), &log)
            .unwrap()
            .unwrap();
        let report_eml = report_msg.to_message_string();
        k9::snapshot!(
            &report_eml,
            r#"
Content-Type: multipart/report;\r
\tboundary="report-boundary";\r
\treport-type="delivery-status"\r
Subject: Returned mail\r
Mime-Version: 1.0\r
Message-ID: <UUID@mta1.example.com>\r
To: sender@sender.example.com\r
From: Mail Delivery Subsystem <mailer-daemon@mta1.example.com>\r
\r
--report-boundary\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
\r
The message was received at Tue, 1 Jul 2003 08:52:37 +0000\r
from sender@sender.example.com and addressed to recip@target.example.com.\r
While communicating with target.example.com (42.42.42.42):\r
Response: 550 5.7.1 no thanks\r
\r
The message will be deleted from the queue.\r
No further attempts will be made to deliver it.\r
--report-boundary\r
Content-Type: message/delivery-status;\r
\tcharset="us-ascii"\r
\r
Reporting-MTA: dns; mta1.example.com\r
Arrival-Date: Tue, 1 Jul 2003 08:52:37 +0000\r
\r
Final-Recipient: rfc822;recip@target.example.com\r
Action: failed\r
Status: 5.7.1 no thanks\r
Remote-MTA: dns; target.example.com\r
Diagnostic-Code: smtp; 550 5.7.1 no thanks\r
Last-Attempt-Date: Tue, 1 Jul 2003 10:52:37 +0000\r
\r
--report-boundary--\r

"#
        );

        let round_trip = Report::parse(report_eml.as_bytes()).unwrap().unwrap();
        k9::snapshot!(
            &round_trip,
            r#"
Report {
    per_message: PerMessageReportEntry {
        original_envelope_id: None,
        reporting_mta: RemoteMta {
            mta_type: "dns",
            name: "mta1.example.com",
        },
        dsn_gateway: None,
        received_from_mta: None,
        arrival_date: Some(
            2003-07-01T08:52:37Z,
        ),
        extensions: {},
    },
    per_recipient: [
        PerRecipientReportEntry {
            final_recipient: Recipient {
                recipient_type: "rfc822",
                recipient: "recip@target.example.com",
            },
            action: Failed,
            status: ReportStatus {
                class: 5,
                subject: 7,
                detail: 1,
                comment: Some(
                    "no thanks",
                ),
            },
            original_recipient: None,
            remote_mta: Some(
                RemoteMta {
                    mta_type: "dns",
                    name: "target.example.com",
                },
            ),
            diagnostic_code: Some(
                DiagnosticCode {
                    diagnostic_type: "smtp",
                    diagnostic: "550 5.7.1 no thanks",
                },
            ),
            last_attempt_date: Some(
                2003-07-01T10:52:37Z,
            ),
            final_log_id: None,
            will_retry_until: None,
            extensions: {},
        },
    ],
    original_message: None,
}
"#
        );
    }

    #[test]
    fn rfc3464_1() {
        let result = Report::parse(include_bytes!("../data/rfc3464/1.eml")).unwrap();
        k9::snapshot!(
            &result,
            r#"
Some(
    Report {
        per_message: PerMessageReportEntry {
            original_envelope_id: None,
            reporting_mta: RemoteMta {
                mta_type: "dns",
                name: "cs.utk.edu",
            },
            dsn_gateway: None,
            received_from_mta: None,
            arrival_date: None,
            extensions: {},
        },
        per_recipient: [
            PerRecipientReportEntry {
                final_recipient: Recipient {
                    recipient_type: "rfc822",
                    recipient: "louisl@larry.slip.umd.edu",
                },
                action: Failed,
                status: ReportStatus {
                    class: 4,
                    subject: 0,
                    detail: 0,
                    comment: None,
                },
                original_recipient: Some(
                    Recipient {
                        recipient_type: "rfc822",
                        recipient: "louisl@larry.slip.umd.edu",
                    },
                ),
                remote_mta: None,
                diagnostic_code: Some(
                    DiagnosticCode {
                        diagnostic_type: "smtp",
                        diagnostic: "426 connection timed out",
                    },
                ),
                last_attempt_date: Some(
                    1994-07-07T21:15:49Z,
                ),
                final_log_id: None,
                will_retry_until: None,
                extensions: {},
            },
        ],
        original_message: Some(
            "[original message goes here]

",
        ),
    },
)
"#
        );

        let report = result.unwrap();

        assert_eq!(
            report.per_message.to_string(),
            "Reporting-MTA: dns; cs.utk.edu\r\n"
        );
        assert_eq!(
            report.per_recipient[0].to_string(),
            "Original-Recipient: rfc822;louisl@larry.slip.umd.edu\r\n\
            Final-Recipient: rfc822;louisl@larry.slip.umd.edu\r\n\
            Action: failed\r\n\
            Status: 4.0.0\r\n\
            Diagnostic-Code: smtp; 426 connection timed out\r\n\
            Last-Attempt-Date: Thu, 7 Jul 1994 21:15:49 +0000\r\n"
        );
    }

    #[test]
    fn rfc3464_2() {
        let result = Report::parse(include_bytes!("../data/rfc3464/2.eml")).unwrap();
        k9::snapshot!(
            result,
            r#"
Some(
    Report {
        per_message: PerMessageReportEntry {
            original_envelope_id: None,
            reporting_mta: RemoteMta {
                mta_type: "dns",
                name: "cs.utk.edu",
            },
            dsn_gateway: None,
            received_from_mta: None,
            arrival_date: None,
            extensions: {},
        },
        per_recipient: [
            PerRecipientReportEntry {
                final_recipient: Recipient {
                    recipient_type: "rfc822",
                    recipient: "arathib@vnet.ibm.com",
                },
                action: Failed,
                status: ReportStatus {
                    class: 5,
                    subject: 0,
                    detail: 0,
                    comment: Some(
                        "(permanent failure)",
                    ),
                },
                original_recipient: Some(
                    Recipient {
                        recipient_type: "rfc822",
                        recipient: "arathib@vnet.ibm.com",
                    },
                ),
                remote_mta: Some(
                    RemoteMta {
                        mta_type: "dns",
                        name: "vnet.ibm.com",
                    },
                ),
                diagnostic_code: Some(
                    DiagnosticCode {
                        diagnostic_type: "smtp",
                        diagnostic: "550 'arathib@vnet.IBM.COM' is not a registered gateway user",
                    },
                ),
                last_attempt_date: None,
                final_log_id: None,
                will_retry_until: None,
                extensions: {},
            },
            PerRecipientReportEntry {
                final_recipient: Recipient {
                    recipient_type: "rfc822",
                    recipient: "johnh@hpnjld.njd.hp.com",
                },
                action: Delayed,
                status: ReportStatus {
                    class: 4,
                    subject: 0,
                    detail: 0,
                    comment: Some(
                        "(hpnjld.njd.jp.com: host name lookup failure)",
                    ),
                },
                original_recipient: Some(
                    Recipient {
                        recipient_type: "rfc822",
                        recipient: "johnh@hpnjld.njd.hp.com",
                    },
                ),
                remote_mta: None,
                diagnostic_code: None,
                last_attempt_date: None,
                final_log_id: None,
                will_retry_until: None,
                extensions: {},
            },
            PerRecipientReportEntry {
                final_recipient: Recipient {
                    recipient_type: "rfc822",
                    recipient: "wsnell@sdcc13.ucsd.edu",
                },
                action: Failed,
                status: ReportStatus {
                    class: 5,
                    subject: 0,
                    detail: 0,
                    comment: None,
                },
                original_recipient: Some(
                    Recipient {
                        recipient_type: "rfc822",
                        recipient: "wsnell@sdcc13.ucsd.edu",
                    },
                ),
                remote_mta: Some(
                    RemoteMta {
                        mta_type: "dns",
                        name: "sdcc13.ucsd.edu",
                    },
                ),
                diagnostic_code: Some(
                    DiagnosticCode {
                        diagnostic_type: "smtp",
                        diagnostic: "550 user unknown",
                    },
                ),
                last_attempt_date: None,
                final_log_id: None,
                will_retry_until: None,
                extensions: {},
            },
        ],
        original_message: Some(
            "[original message goes here]

",
        ),
    },
)
"#
        );
    }

    #[test]
    fn rfc3464_3() {
        let result = Report::parse(include_bytes!("../data/rfc3464/3.eml")).unwrap();
        k9::snapshot!(
            result,
            r#"
Some(
    Report {
        per_message: PerMessageReportEntry {
            original_envelope_id: None,
            reporting_mta: RemoteMta {
                mta_type: "mailbus",
                name: "SYS30",
            },
            dsn_gateway: None,
            received_from_mta: None,
            arrival_date: None,
            extensions: {},
        },
        per_recipient: [
            PerRecipientReportEntry {
                final_recipient: Recipient {
                    recipient_type: "unknown",
                    recipient: "nair_s",
                },
                action: Failed,
                status: ReportStatus {
                    class: 5,
                    subject: 0,
                    detail: 0,
                    comment: Some(
                        "(unknown permanent failure)",
                    ),
                },
                original_recipient: None,
                remote_mta: None,
                diagnostic_code: None,
                last_attempt_date: None,
                final_log_id: None,
                will_retry_until: None,
                extensions: {},
            },
        ],
        original_message: None,
    },
)
"#
        );
    }

    #[test]
    fn rfc3464_4() {
        let result = Report::parse(include_bytes!("../data/rfc3464/4.eml")).unwrap();
        k9::snapshot!(
            result,
            r#"
Some(
    Report {
        per_message: PerMessageReportEntry {
            original_envelope_id: None,
            reporting_mta: RemoteMta {
                mta_type: "dns",
                name: "sun2.nsfnet-relay.ac.uk",
            },
            dsn_gateway: None,
            received_from_mta: None,
            arrival_date: None,
            extensions: {},
        },
        per_recipient: [
            PerRecipientReportEntry {
                final_recipient: Recipient {
                    recipient_type: "rfc822",
                    recipient: "thomas@de-montfort.ac.uk",
                },
                action: Delayed,
                status: ReportStatus {
                    class: 4,
                    subject: 0,
                    detail: 0,
                    comment: Some(
                        "(unknown temporary failure)",
                    ),
                },
                original_recipient: None,
                remote_mta: None,
                diagnostic_code: None,
                last_attempt_date: None,
                final_log_id: None,
                will_retry_until: None,
                extensions: {},
            },
        ],
        original_message: None,
    },
)
"#
        );
    }

    #[test]
    fn rfc3464_5() {
        let result = Report::parse(include_bytes!("../data/rfc3464/5.eml")).unwrap();
        k9::snapshot!(
            result,
            r#"
Some(
    Report {
        per_message: PerMessageReportEntry {
            original_envelope_id: None,
            reporting_mta: RemoteMta {
                mta_type: "dns",
                name: "mx-by.bbox.fr",
            },
            dsn_gateway: None,
            received_from_mta: None,
            arrival_date: Some(
                2025-01-29T16:36:51Z,
            ),
            extensions: {
                "x-postfix-queue-id": [
                    "897DAC0",
                ],
                "x-postfix-sender": [
                    "rfc822; user@example.com",
                ],
            },
        },
        per_recipient: [
            PerRecipientReportEntry {
                final_recipient: Recipient {
                    recipient_type: "rfc822",
                    recipient: "recipient@domain.com",
                },
                action: Failed,
                status: ReportStatus {
                    class: 5,
                    subject: 0,
                    detail: 0,
                    comment: None,
                },
                original_recipient: Some(
                    Recipient {
                        recipient_type: "rfc822",
                        recipient: "recipient@domain.com",
                    },
                ),
                remote_mta: Some(
                    RemoteMta {
                        mta_type: "dns",
                        name: "lmtp.cs.dolmen.bouyguestelecom.fr",
                    },
                ),
                diagnostic_code: Some(
                    DiagnosticCode {
                        diagnostic_type: "smtp",
                        diagnostic: "552 <recipient@domain.com> rejected: over quota",
                    },
                ),
                last_attempt_date: None,
                final_log_id: None,
                will_retry_until: None,
                extensions: {},
            },
        ],
        original_message: Some(
            "[original message goes here]

",
        ),
    },
)
"#
        );
    }
}
