//! This module parses out RFC3464 delivery status reports
//! from an email message
use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use mailparse::{parse_headers, parse_mail, ParsedMail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Copy, Clone, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReportAction {
    Failed,
    Delayed,
    Delivered,
    Relayed,
    Expanded,
}

impl ReportAction {
    fn parse(input: &str) -> anyhow::Result<Self> {
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

impl ReportStatus {
    fn parse(input: &str) -> anyhow::Result<Self> {
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

impl RemoteMta {
    fn parse(input: &str) -> anyhow::Result<Self> {
        let (mta_type, name) = input
            .split_once(";")
            .ok_or_else(|| anyhow!("expected 'name-type; name', got {input}"))?;
        Ok(Self {
            mta_type: mta_type.trim().to_string(),
            name: name.trim().to_string(),
        })
    }

    fn from_extension_field(
        name: &str,
        extensions: &mut HashMap<String, String>,
    ) -> anyhow::Result<Option<Self>> {
        let field = match extensions.remove(name) {
            Some(f) => f,
            None => return Ok(None),
        };

        Ok(Some(Self::parse(&field).with_context(|| name.to_string())?))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct Recipient {
    pub recipient_type: String,
    pub recipient: String,
}
impl Recipient {
    fn parse(input: &str) -> anyhow::Result<Self> {
        let (recipient_type, recipient) = input
            .split_once(";")
            .ok_or_else(|| anyhow!("expected 'recipient-type; recipient', got {input}"))?;
        Ok(Self {
            recipient_type: recipient_type.trim().to_string(),
            recipient: recipient.trim().to_string(),
        })
    }

    fn from_extensions(
        name: &str,
        extensions: &mut HashMap<String, String>,
    ) -> anyhow::Result<Option<Self>> {
        let field = match extensions.remove(name) {
            Some(f) => f,
            None => return Ok(None),
        };

        let addr = Self::parse(&field)?;
        Ok(Some(addr))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct DiagnosticCode {
    pub diagnostic_type: String,
    pub diagnostic: String,
}
impl DiagnosticCode {
    fn parse(input: &str) -> anyhow::Result<Self> {
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
    pub extensions: HashMap<String, String>,
}

impl PerRecipientReportEntry {
    fn parse(part: &str) -> anyhow::Result<Self> {
        let (headers, _) = parse_headers(part.as_bytes())?;
        let mut extensions = HashMap::new();

        for hdr in headers {
            let name = hdr.get_key_ref().to_ascii_lowercase();
            extensions.insert(name, hdr.get_value_utf8()?);
        }

        let original_recipient = Recipient::from_extensions("original-recipient", &mut extensions)?;
        let final_recipient = Recipient::from_extensions("final-recipient", &mut extensions)?
            .ok_or_else(|| anyhow!("required final-recipient header is missing"))?;
        let remote_mta = RemoteMta::from_extension_field("remote-mta", &mut extensions)?;

        let last_attempt_date = date_from_extensions("last-attempt-date", &mut extensions)?;
        let will_retry_until = date_from_extensions("will-retry-until", &mut extensions)?;
        let final_log_id = extensions.remove("final-log-id");

        let action = match extensions.remove("action") {
            Some(a) => ReportAction::parse(&a)?,
            None => anyhow::bail!("required action is missing"),
        };
        let status = match extensions.remove("status") {
            Some(a) => ReportStatus::parse(&a)?,
            None => anyhow::bail!("required status is missing"),
        };
        let diagnostic_code = match extensions.remove("diagnostic-code") {
            Some(a) => Some(DiagnosticCode::parse(&a)?),
            None => None,
        };

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
    pub extensions: HashMap<String, String>,
}

fn date_from_extensions(
    name: &str,
    extensions: &mut HashMap<String, String>,
) -> anyhow::Result<Option<DateTime<Utc>>> {
    let field = match extensions.remove(name) {
        Some(f) => f,
        None => return Ok(None),
    };

    let date = DateTime::parse_from_rfc2822(&field)?;
    Ok(Some(date.into()))
}

impl PerMessageReportEntry {
    fn parse(part: &str) -> anyhow::Result<Self> {
        let (headers, _) = parse_headers(part.as_bytes())?;
        let mut extensions = HashMap::new();

        for hdr in headers {
            let name = hdr.get_key_ref().to_ascii_lowercase();
            extensions.insert(name, hdr.get_value_utf8()?);
        }

        let reporting_mta = RemoteMta::from_extension_field("reporting-mta", &mut extensions)?
            .ok_or_else(|| anyhow!("required Reporting-MTA field is missing"))?;

        let original_envelope_id = extensions.remove("original-envelope-id");
        let dsn_gateway = RemoteMta::from_extension_field("dsn-gateway", &mut extensions)?;
        let received_from_mta =
            RemoteMta::from_extension_field("received-from-mta", &mut extensions)?;

        let arrival_date = date_from_extensions("arrival-date", &mut extensions)?;

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
}

impl Report {
    pub fn parse(input: &[u8]) -> anyhow::Result<Option<Self>> {
        let mail = parse_mail(input)?;

        if mail.ctype.mimetype != "multipart/report" {
            return Ok(None);
        }

        for part in &mail.subparts {
            if part.ctype.mimetype == "message/delivery-status"
                || part.ctype.mimetype == "message/global-delivery-status"
            {
                return Ok(Some(Self::parse_inner(part)?));
            }
        }

        anyhow::bail!("delivery-status part missing");
    }

    fn parse_inner(part: &ParsedMail) -> anyhow::Result<Self> {
        let body = part.get_body()?;
        let body = body.replace("\r\n", "\n");
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
    },
)
"#
        );
    }
}
