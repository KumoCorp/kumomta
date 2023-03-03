//! ARF reports
use crate::rfc3464::RemoteMta;
use anyhow::anyhow;
use chrono::{DateTime, Utc};
use mailparse::{parse_headers, parse_mail, ParsedMail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct ARFReport {
    pub feedback_type: String,
    pub user_agent: String,
    pub version: u32,

    #[serde(default)]
    pub arrival_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub incidents: Option<u32>,
    #[serde(default)]
    pub original_envelope_id: Option<String>,
    #[serde(default)]
    pub original_mail_from: Option<String>,
    #[serde(default)]
    pub reporting_mta: Option<RemoteMta>,
    #[serde(default)]
    pub source_ip: Option<String>,

    #[serde(default)]
    pub authentication_results: Vec<String>,
    #[serde(default)]
    pub original_rcpto_to: Vec<String>,
    #[serde(default)]
    pub reported_domain: Vec<String>,
    #[serde(default)]
    pub reported_uri: Vec<String>,

    pub extensions: HashMap<String, Vec<String>>,

    pub original_message: Option<String>,
}

impl ARFReport {
    pub fn parse(input: &[u8]) -> anyhow::Result<Option<Self>> {
        let mail = parse_mail(input)?;

        if mail.ctype.mimetype != "multipart/report" {
            return Ok(None);
        }
        if mail.ctype.params.get("report-type").map(|s| s.as_str()) != Some("feedback-report") {
            return Ok(None);
        }

        let mut original_message = None;

        for part in &mail.subparts {
            if part.ctype.mimetype == "message/rfc822"
                || part.ctype.mimetype == "text/rfc822-headers"
            {
                let (_headers, offset) = parse_headers(part.raw_bytes)?;
                original_message =
                    Some(String::from_utf8_lossy(&part.raw_bytes[offset..]).replace("\r\n", "\n"));
            }
        }

        for part in &mail.subparts {
            if part.ctype.mimetype == "message/feedback-report" {
                return Ok(Some(Self::parse_inner(part, original_message)?));
            }
        }

        anyhow::bail!("feedback-report part missing");
    }

    fn parse_inner(part: &ParsedMail, original_message: Option<String>) -> anyhow::Result<Self> {
        let body = part.get_body()?;
        let mut extensions = extract_headers(body.as_bytes())?;

        let feedback_type = extract_single_req("feedback-type", &mut extensions)?;
        let user_agent = extract_single_req("user-agent", &mut extensions)?;
        let version = extract_single_req("version", &mut extensions)?;
        let arrival_date =
            extract_single_conv::<DateTimeRfc2822, DateTime<Utc>>("arrival-date", &mut extensions)?;
        let incidents = extract_single("incidents", &mut extensions)?;
        let original_envelope_id = extract_single("original-envelope-id", &mut extensions)?;
        let original_mail_from = extract_single("original-mail-from", &mut extensions)?;
        let reporting_mta = extract_single("reporting-mta", &mut extensions)?;
        let source_ip = extract_single("source-ip", &mut extensions)?;
        let authentication_results = extract_multiple("authentication-results", &mut extensions)?;
        let original_rcpto_to = extract_multiple("original-rcpt-to", &mut extensions)?;
        let reported_domain = extract_multiple("reported-domain", &mut extensions)?;
        let reported_uri = extract_multiple("reported-uri", &mut extensions)?;

        Ok(Self {
            feedback_type,
            user_agent,
            version,
            arrival_date,
            incidents,
            original_envelope_id,
            original_mail_from,
            reporting_mta,
            source_ip,
            authentication_results,
            original_rcpto_to,
            reported_domain,
            reported_uri,
            extensions,
            original_message,
        })
    }
}

pub(crate) fn extract_headers(part: &[u8]) -> anyhow::Result<HashMap<String, Vec<String>>> {
    let (headers, _) = parse_headers(part)?;

    let mut extensions = HashMap::new();

    for hdr in headers {
        let name = hdr.get_key_ref().to_ascii_lowercase();
        extensions
            .entry(name)
            .or_insert_with(|| vec![])
            .push(hdr.get_value_utf8()?);
    }
    Ok(extensions)
}

pub(crate) struct DateTimeRfc2822(pub DateTime<Utc>);

impl FromStr for DateTimeRfc2822 {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> anyhow::Result<Self> {
        let date = DateTime::parse_from_rfc2822(input)?;
        Ok(Self(date.into()))
    }
}

impl Into<DateTime<Utc>> for DateTimeRfc2822 {
    fn into(self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) fn extract_single_req<R>(
    name: &str,
    extensions: &mut HashMap<String, Vec<String>>,
) -> anyhow::Result<R>
where
    R: FromStr,
    <R as FromStr>::Err: std::fmt::Display,
{
    extract_single(name, extensions)?
        .ok_or_else(|| anyhow!("required header {name} is not present"))
}

pub(crate) fn extract_single<R>(
    name: &str,
    extensions: &mut HashMap<String, Vec<String>>,
) -> anyhow::Result<Option<R>>
where
    R: FromStr,
    <R as FromStr>::Err: std::fmt::Display,
{
    match extensions.remove(name) {
        Some(mut hdrs) if hdrs.len() == 1 => {
            let value = hdrs.remove(0);
            let converted = value
                .parse::<R>()
                .map_err(|err| anyhow!("failed to convert '{value}': {err:#}"))?;
            Ok(Some(converted))
        }
        Some(_) => anyhow::bail!("header {name} should have only a single value"),
        None => Ok(None),
    }
}

pub(crate) fn extract_single_conv<R, T>(
    name: &str,
    extensions: &mut HashMap<String, Vec<String>>,
) -> anyhow::Result<Option<T>>
where
    R: FromStr,
    <R as FromStr>::Err: std::fmt::Display,
    R: Into<T>,
{
    Ok(extract_single::<R>(name, extensions)?.map(|v| v.into()))
}

pub(crate) fn extract_multiple<R>(
    name: &str,
    extensions: &mut HashMap<String, Vec<String>>,
) -> anyhow::Result<Vec<R>>
where
    R: FromStr,
    <R as FromStr>::Err: std::fmt::Display,
{
    match extensions.remove(name) {
        Some(hdrs) => {
            let mut results = vec![];
            for h in hdrs {
                let converted = h
                    .parse::<R>()
                    .map_err(|err| anyhow!("failed to convert {h}: {err:#}"))?;
                results.push(converted);
            }
            Ok(results)
        }
        None => Ok(vec![]),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn rfc5965_1() {
        let result = ARFReport::parse(include_bytes!("../data/rfc5965/1.eml")).unwrap();
        k9::snapshot!(
            result,
            r#"
Some(
    ARFReport {
        feedback_type: "abuse",
        user_agent: "SomeGenerator/1.0",
        version: 1,
        arrival_date: None,
        incidents: None,
        original_envelope_id: None,
        original_mail_from: None,
        reporting_mta: None,
        source_ip: None,
        authentication_results: [],
        original_rcpto_to: [],
        reported_domain: [],
        reported_uri: [],
        extensions: {},
        original_message: Some(
            "Received: from mailserver.example.net
    (mailserver.example.net [192.0.2.1])
    by example.com with ESMTP id M63d4137594e46;
    Thu, 08 Mar 2005 14:00:00 -0400
From: <somespammer@example.net>
To: <Undisclosed Recipients>
Subject: Earn money
MIME-Version: 1.0
Content-type: text/plain
Message-ID: 8787KJKJ3K4J3K4J3K4J3.mail@example.net
Date: Thu, 02 Sep 2004 12:31:03 -0500

Spam Spam Spam
Spam Spam Spam
Spam Spam Spam
Spam Spam Spam
",
        ),
    },
)
"#
        );
    }

    #[test]
    fn rfc5965_2() {
        let result = ARFReport::parse(include_bytes!("../data/rfc5965/2.eml")).unwrap();
        k9::snapshot!(
            result,
            r#"
Some(
    ARFReport {
        feedback_type: "abuse",
        user_agent: "SomeGenerator/1.0",
        version: 1,
        arrival_date: Some(
            2005-03-08T18:00:00Z,
        ),
        incidents: None,
        original_envelope_id: None,
        original_mail_from: Some(
            "<somespammer@example.net>",
        ),
        reporting_mta: Some(
            RemoteMta {
                mta_type: "dns",
                name: "mail.example.com",
            },
        ),
        source_ip: Some(
            "192.0.2.1",
        ),
        authentication_results: [
            "mail.example.com; spf=fail smtp.mail=somespammer@example.com",
        ],
        original_rcpto_to: [
            "<user@example.com>",
        ],
        reported_domain: [
            "example.net",
        ],
        reported_uri: [
            "http://example.net/earn_money.html",
            "mailto:user@example.com",
        ],
        extensions: {
            "removal-recipient": [
                "user@example.com",
            ],
        },
        original_message: Some(
            "From: <somespammer@example.net>
Received: from mailserver.example.net (mailserver.example.net
    [192.0.2.1]) by example.com with ESMTP id M63d4137594e46;
    Tue, 08 Mar 2005 14:00:00 -0400

To: <Undisclosed Recipients>
Subject: Earn money
MIME-Version: 1.0
Content-type: text/plain
Message-ID: 8787KJKJ3K4J3K4J3K4J3.mail@example.net
Date: Thu, 02 Sep 2004 12:31:03 -0500

Spam Spam Spam
Spam Spam Spam
Spam Spam Spam
Spam Spam Spam
",
        ),
    },
)
"#
        );
    }
}
