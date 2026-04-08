use crate::types::feedback_address::FeedbackAddress;
use crate::types::format::Format;
use crate::types::mode::Mode;
use crate::types::policy::Policy;
use crate::types::report_failure::ReportFailure;
use crate::types::results::{Disposition, DispositionWithContext};
use crate::{DmarcContext, SenderDomainAlignment};
use bstr::ByteSlice;
use std::str::FromStr;

#[derive(Debug)]
pub struct Record {
    pub align_dkim: Mode,
    pub align_spf: Mode,
    report_failure: ReportFailure,
    pub policy: Policy,
    rate: u8,
    format: Format,
    interval: u32,
    aggregate_feedback: Vec<FeedbackAddress>,
    message_failure: Vec<FeedbackAddress>,
    subdomain_policy: Option<Policy>,
}

impl Record {
    pub(crate) async fn evaluate(
        &self,
        cx: &DmarcContext<'_>,
        sender_location: SenderDomainAlignment,
    ) -> DispositionWithContext {
        if rand::random::<u8>() % 100 >= self.rate {
            return DispositionWithContext {
                result: Disposition::Pass,
                context: format!("sampled_out due to pct={}", self.rate),
            };
        }

        let mut dkim_aligned = false;
        let mut spf_aligned = false;
        let mut dkim_error = None;
        let mut spf_error = None;

        match self.align_dkim {
            Mode::Relaxed => {
                for dkim in cx.dkim {
                    if !auth_result_is_pass(dkim) {
                        continue;
                    }

                    if let Some(result) = dkim.get("header.d".as_bytes()) {
                        if let Ok(result_str) = result.to_str() {
                            if is_relaxed_aligned(cx.from_domain, result_str) {
                                dkim_aligned = true;
                                break;
                            }

                            dkim_error.get_or_insert("DMARC: DKIM relaxed check failed");
                        }
                    } else {
                        dkim_error.get_or_insert("DMARC: DKIM signature missing 'd=' tag");
                    }
                }
            }
            Mode::Strict => {
                for dkim in cx.dkim {
                    if !auth_result_is_pass(dkim) {
                        continue;
                    }

                    if let Some(result) = dkim.get("header.d".as_bytes()) {
                        if let Ok(result_str) = result.to_str() {
                            if is_strict_aligned(cx.from_domain, result_str) {
                                dkim_aligned = true;
                                break;
                            }

                            dkim_error.get_or_insert("DMARC: DKIM strict check failed");
                        }
                    } else {
                        dkim_error.get_or_insert("DMARC: DKIM signature missing 'd=' tag");
                    }
                }
            }
        }

        match self.align_spf {
            Mode::Relaxed => {
                if let Some(spf) = cx.spf {
                    if auth_result_is_pass(spf) {
                        if let Some(spf_domain) = spf_alignment_domain(spf) {
                            if is_relaxed_aligned(cx.from_domain, spf_domain) {
                                spf_aligned = true;
                            } else {
                                spf_error.get_or_insert("DMARC: SPF relaxed check failed");
                            }
                        } else {
                            spf_error.get_or_insert(
                                "DMARC: SPF pass result missing smtp.mailfrom/smtp.helo",
                            );
                        }
                    }
                }
            }
            Mode::Strict => {
                if let Some(spf) = cx.spf {
                    if auth_result_is_pass(spf) {
                        if let Some(spf_domain) = spf_alignment_domain(spf) {
                            if is_strict_aligned(spf_domain, cx.from_domain) {
                                spf_aligned = true;
                            } else {
                                spf_error.get_or_insert("DMARC: SPF strict check failed");
                            }
                        } else {
                            spf_error.get_or_insert(
                                "DMARC: SPF pass result missing smtp.mailfrom/smtp.helo",
                            );
                        }
                    }
                }
            }
        }

        if dkim_aligned || spf_aligned {
            DispositionWithContext {
                result: Disposition::Pass,
                context: "Success".into(),
            }
        } else {
            let context = if let Some(dkim_error) = dkim_error {
                dkim_error.into()
            } else if let Some(spf_error) = spf_error {
                spf_error.into()
            } else {
                "DMARC: neither DKIM nor SPF produced an aligned pass".into()
            };

            DispositionWithContext {
                result: self.select_failure_mode(sender_location),
                context,
            }
        }
    }

    fn select_failure_mode(&self, sender_location: SenderDomainAlignment) -> Disposition {
        match sender_location {
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

fn auth_result_is_pass(
    auth_result: &std::collections::BTreeMap<bstr::BString, bstr::BString>,
) -> bool {
    auth_result
        .get("result".as_bytes())
        .is_some_and(|result| result.eq_ignore_ascii_case(b"pass"))
}

fn spf_alignment_domain<'a>(
    auth_result: &'a std::collections::BTreeMap<bstr::BString, bstr::BString>,
) -> Option<&'a str> {
    auth_result
        .get("smtp.mailfrom".as_bytes())
        .filter(|domain| !domain.is_empty())
        .and_then(|domain| domain.to_str().ok())
        .map(|s| s.split_once('@').map_or(s, |(_, domain)| domain))
        .or_else(|| {
            auth_result
                .get("smtp.helo".as_bytes())
                .filter(|domain| !domain.is_empty())
                .and_then(|domain| domain.to_str().ok())
        })
}

// Relaxed alignment: organizational domain match (covers exact match too since org domain of "example.com" is "example.com")
fn is_relaxed_aligned(from_domain: &str, signing_domain: &str) -> bool {
    psl::domain_str(from_domain)
        .zip(psl::domain_str(signing_domain))
        .is_some_and(|(fd, sd)| fd.eq_ignore_ascii_case(sd))
}

// Strict alignment: exact domain match only.
fn is_strict_aligned(from_domain: &str, signing_domain: &str) -> bool {
    from_domain.eq_ignore_ascii_case(signing_domain)
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
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

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
        const B_2_1: &str = "v=DMARC1; p=none; rua=mailto:dmarc-feedback@example.com;";
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
