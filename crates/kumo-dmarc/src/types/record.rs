use crate::types::feedback_address::FeedbackAddress;
use crate::types::format::Format;
use crate::types::mode::Mode;
use crate::types::policy::Policy;
use crate::types::report_failure::ReportFailure;
use crate::types::results::{Disposition, DispositionWithContext};
use crate::{DmarcContext, SenderDomainAlignment};
use std::str::FromStr;

#[derive(Debug)]
pub struct Record {
    pub align_dkim: Mode,
    pub align_spf: Mode,
    pub report_failure: ReportFailure,
    pub policy: Policy,
    pub subdomain_policy: Option<Policy>,
    pub rate: u8,
    format: Format,
    interval: u32,
    aggregate_feedback: Vec<FeedbackAddress>,
    message_failure: Vec<FeedbackAddress>,
}

impl Record {
    pub(crate) async fn evaluate(
        &self,
        cx: &mut DmarcContext<'_>,
        dmarc_domain: &str,
        sender_domain_alignment: SenderDomainAlignment,
    ) -> DispositionWithContext {
        let mut alignment_failure = None;

        if rand::random::<u8>() % 100 >= self.rate {
            return DispositionWithContext {
                result: Disposition::Pass,
                context: format!("sampled_out due to pct={}", self.rate),
            };
        }
        match self.align_dkim {
            Mode::Relaxed => {
                for dkim in cx.dkim_results {
                    if let Some(result) = dkim.props.get("header.d") {
                        let organizational_domain = psl::domain_str(cx.from_domain);

                        if cx.from_domain != result && organizational_domain != Some(result) {
                            cx.dkim_aligned = super::results::DmarcResult::Fail;

                            alignment_failure = Some(DispositionWithContext {
                                result: self.disposition(sender_domain_alignment),
                                context: "DMARC: DKIM relaxed check failed".into(),
                            });
                        }
                    } else {
                        alignment_failure = Some(DispositionWithContext {
                            result: self.disposition(sender_domain_alignment),
                            context: "DMARC: DKIM signature missing 'd=' tag".into(),
                        });
                    }
                }
            }
            Mode::Strict => {
                for dkim in cx.dkim_results {
                    if let Some(result) = dkim.props.get("header.d") {
                        if cx.from_domain != result {
                            alignment_failure = Some(DispositionWithContext {
                                result: self.disposition(sender_domain_alignment),
                                context: "DMARC: DKIM strict check failed".into(),
                            });
                        }
                    } else {
                        alignment_failure = Some(DispositionWithContext {
                            result: self.disposition(sender_domain_alignment),
                            context: "DMARC: DKIM signature missing 'd=' tag".into(),
                        });
                    }
                }
            }
        }

        match self.align_spf {
            Mode::Relaxed => {
                if let Some(mail_from_domain) = cx.mail_from_domain {
                    let organizational_domain = psl::domain_str(mail_from_domain);

                    if mail_from_domain != cx.from_domain
                        && organizational_domain != Some(cx.from_domain)
                    {
                        alignment_failure = Some(DispositionWithContext {
                            result: self.disposition(sender_domain_alignment),
                            context: "DMARC: SPF relaxed check failed".into(),
                        });
                    }
                }
            }
            Mode::Strict => {
                if let Some(mail_from_domain) = cx.mail_from_domain {
                    if mail_from_domain != cx.from_domain {
                        alignment_failure = Some(DispositionWithContext {
                            result: self.disposition(sender_domain_alignment),
                            context: "DMARC: SPF strict check failed".into(),
                        });
                    }
                }
            }
        }

        if let Some(alignment_failure) = alignment_failure {
            let _ = cx.report_error(
                self,
                dmarc_domain,
                sender_domain_alignment,
                &alignment_failure.context,
            )
            .await;
            return DispositionWithContext {
                result: alignment_failure.result,
                context: alignment_failure.context,
            };
        }

        DispositionWithContext {
            result: Disposition::Pass,
            context: "Success".into(),
        }
    }

    pub(crate) fn policy_result(&self, sender_domain_alignment: SenderDomainAlignment) -> Policy {
        match sender_domain_alignment {
            SenderDomainAlignment::OrganizationalDomain => {
                if let Some(policy) = self.subdomain_policy {
                    policy
                } else {
                    self.policy
                }
            }
            SenderDomainAlignment::Exact => self.policy,
        }
    }

    fn disposition(&self, sender_domain_alignment: SenderDomainAlignment) -> Disposition {
        match sender_domain_alignment {
            SenderDomainAlignment::OrganizationalDomain => {
                if let Some(policy) = self.subdomain_policy {
                    policy.into()
                } else {
                    self.policy.into()
                }
            }
            SenderDomainAlignment::Exact => self.policy.into(),
        }
    }
}

impl FromStr for Record {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut new = Self {
            align_dkim: Mode::Relaxed,
            align_spf: Mode::Relaxed,
            report_failure: ReportFailure::default(),
            policy: Policy::None,
            rate: 100,
            format: Format::Afrf,
            interval: 86400,
            aggregate_feedback: Vec::new(),
            message_failure: Vec::new(),
            subdomain_policy: None,
        };

        let (mut version, mut policy) = (false, false);
        for part in s.split(';') {
            let Some((key, value)) = part.split_once('=') else {
                return Err(format!("invalid part {part:?}"));
            };

            let (key, value) = (key.trim(), value.trim());
            if !version {
                match (key, value) {
                    ("v", "DMARC1") => {
                        version = true;
                        continue;
                    }
                    _ => return Err(format!("invalid key {key:?}")),
                }
            }

            match key {
                "p" => {
                    new.policy = Policy::from_str(value)?;
                    policy = true;
                }
                "adkim" => new.align_dkim = Mode::from_str(value)?,
                "aspf" => new.align_spf = Mode::from_str(value)?,
                "fo" => new.report_failure = ReportFailure::from_str(value)?,
                "pct" => {
                    new.rate = u8::from_str(value)
                        .map_err(|_| format!("invalid value {value:?} for pct"))?
                }
                "rf" => new.format = Format::from_str(value)?,
                "ri" => {
                    new.interval = u32::from_str(value)
                        .map_err(|_| format!("invalid value {value:?} for ri"))?
                }
                "rua" => {
                    for addr in value.split(',') {
                        new.aggregate_feedback
                            .push(FeedbackAddress::from_str(addr)?);
                    }
                }
                "ruf" => {
                    for addr in value.split(',') {
                        new.message_failure.push(FeedbackAddress::from_str(addr)?);
                    }
                }
                "sp" => new.subdomain_policy = Some(Policy::from_str(value)?),
                _ => return Err(format!("invalid key {key:?}")),
            }
        }

        if policy {
            Ok(new)
        } else {
            Err(format!("missing policy in {s:?}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_b_2_1() {
        // https://www.rfc-editor.org/rfc/rfc7489#appendix-B.2.1
        const B_2_1: &str = "v=DMARC1; p=none; rua=mailto:dmarc-feedback@example.com";
        let record = Record::from_str(B_2_1).unwrap();
        assert_eq!(record.policy, Policy::None);
        assert_eq!(record.rate, 100);
        assert_eq!(
            record.aggregate_feedback[0].uri,
            "mailto:dmarc-feedback@example.com",
        );
        assert_eq!(record.aggregate_feedback[0].size, None);
    }

    #[test]
    fn parse_b_2_2() {
        // https://www.rfc-editor.org/rfc/rfc7489#appendix-B.2.2
        const B_2_2: &str = "v=DMARC1; p=none; rua=mailto:dmarc-feedback@example.com; ruf=mailto:auth-reports@example.com";
        let record = Record::from_str(B_2_2).unwrap();
        assert_eq!(record.policy, Policy::None);
        assert_eq!(
            record.aggregate_feedback[0].uri,
            "mailto:dmarc-feedback@example.com",
        );
        assert_eq!(
            record.message_failure[0].uri,
            "mailto:auth-reports@example.com",
        );
    }

    #[test]
    fn parse_b_2_4() {
        // https://www.rfc-editor.org/rfc/rfc7489#appendix-B.2.4
        const B_2_4: &str = r#"v=DMARC1; p=quarantine;
                       rua=mailto:dmarc-feedback@example.com,
                       mailto:tld-test@thirdparty.example.net!10m;
                       pct=25"#;
        let record = Record::from_str(B_2_4).unwrap();
        assert_eq!(record.policy, Policy::Quarantine);
        assert_eq!(record.rate, 25);
        assert_eq!(
            record.aggregate_feedback[0].uri,
            "mailto:dmarc-feedback@example.com",
        );
        assert_eq!(
            record.aggregate_feedback[1].uri,
            "mailto:tld-test@thirdparty.example.net",
        );
        assert_eq!(record.aggregate_feedback[1].size, Some(10_485_760));
    }
}
