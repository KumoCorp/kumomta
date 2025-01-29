//! This module parses out RFC3464 delivery status reports
//! from an email message
use crate::rfc5965::{
    extract_headers, extract_single, extract_single_conv, extract_single_req, DateTimeRfc2822,
};
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
        while let Some(part) = parts.next() {
            let part = PerRecipientReportEntry::parse(part)?;
            per_recipient.push(part);
        }

        Ok(Self {
            per_message,
            per_recipient,
            original_message,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn rfc3464_1() {
        let result = Report::parse(include_bytes!("../data/rfc3464/1.eml")).unwrap();
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
}
