//! ARF reports
use crate::rfc3464::{content_type, RemoteMta};
use anyhow::anyhow;
use chrono::{DateTime, Utc};
use mailparsing::{Header, HeaderParseResult, MimePart};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct ARFReport {
    pub feedback_type: String,
    pub user_agent: String,
    pub version: String,

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

    pub extensions: BTreeMap<String, Vec<String>>,

    pub original_message: Option<String>,
    pub supplemental_trace: Option<serde_json::Value>,
}

impl ARFReport {
    pub fn parse(input: &[u8]) -> anyhow::Result<Option<Self>> {
        let mail = MimePart::parse(input)?;
        let ct = mail.headers().content_type()?;
        let ct = match ct {
            None => return Ok(None),
            Some(ct) => ct,
        };

        if ct.value != "multipart/report" {
            return Ok(None);
        }

        if ct.get("report-type").as_deref() != Some("feedback-report") {
            return Ok(None);
        }

        let mut original_message = None;
        let mut supplemental_trace = None;

        for part in mail.child_parts() {
            let ct = content_type(part);
            let ct = ct.as_deref();
            if ct == Some("message/rfc822") || ct == Some("text/rfc822-headers") {
                if let Ok(HeaderParseResult { headers, .. }) =
                    Header::parse_headers(part.raw_body())
                {
                    // Look for x-headers that might be our supplemental trace headers
                    for hdr in headers.iter() {
                        if !(hdr.get_name().starts_with("X-") || hdr.get_name().starts_with("x-")) {
                            continue;
                        }
                        if let Ok(decoded) =
                            data_encoding::BASE64.decode(hdr.get_raw_value().as_bytes())
                        {
                            #[derive(Deserialize)]
                            struct Wrap {
                                #[serde(rename = "_@_")]
                                marker: String,
                                #[serde(flatten)]
                                payload: serde_json::Value,
                            }
                            if let Ok(obj) = serde_json::from_slice::<Wrap>(&decoded) {
                                // Sanity check that it is our encoded data, rather than
                                // some other random header that may have been inserted
                                // somewhere along the way
                                if obj.marker == "\\_/" {
                                    supplemental_trace.replace(obj.payload);
                                    break;
                                }
                            }
                        }
                    }
                }

                original_message = Some(part.raw_body().replace("\r\n", "\n"));
            }
        }

        for part in mail.child_parts() {
            let ct = content_type(part);
            let ct = ct.as_deref();
            if ct == Some("message/feedback-report") {
                return Ok(Some(Self::parse_inner(
                    part,
                    original_message,
                    supplemental_trace,
                )?));
            }
        }

        anyhow::bail!("feedback-report part missing");
    }

    fn parse_inner(
        part: &MimePart,
        original_message: Option<String>,
        supplemental_trace: Option<serde_json::Value>,
    ) -> anyhow::Result<Self> {
        let body = part.raw_body();
        let mut extensions = extract_headers(body.as_bytes())?;

        let feedback_type = extract_single_req("feedback-type", &mut extensions)?;
        let user_agent = extract_single_req("user-agent", &mut extensions)?;
        let version = extract_single_req("version", &mut extensions)?;
        let arrival_date = extract_single_conv_fallback::<DateTimeRfc2822, DateTime<Utc>>(
            "arrival-date",
            "received-date",
            &mut extensions,
        );
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
            supplemental_trace,
        })
    }
}

pub(crate) fn extract_headers(part: &[u8]) -> anyhow::Result<BTreeMap<String, Vec<String>>> {
    let HeaderParseResult { headers, .. } = Header::parse_headers(part)?;

    let mut extensions = BTreeMap::new();

    for hdr in headers.iter() {
        let name = hdr.get_name().to_ascii_lowercase();
        extensions
            .entry(name)
            .or_insert_with(std::vec::Vec::new)
            .push(hdr.as_unstructured()?);
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

impl From<DateTimeRfc2822> for DateTime<Utc> {
    fn from(val: DateTimeRfc2822) -> Self {
        val.0
    }
}

pub(crate) fn extract_single_req<R>(
    name: &str,
    extensions: &mut BTreeMap<String, Vec<String>>,
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
    extensions: &mut BTreeMap<String, Vec<String>>,
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
    extensions: &mut BTreeMap<String, Vec<String>>,
) -> anyhow::Result<Option<T>>
where
    R: FromStr,
    <R as FromStr>::Err: std::fmt::Display,
    R: Into<T>,
{
    Ok(extract_single::<R>(name, extensions)?.map(|v| v.into()))
}

pub(crate) fn extract_single_conv_fallback<R, T>(
    name: &str,
    fallback: &str,
    extensions: &mut BTreeMap<String, Vec<String>>,
) -> Option<T>
where
    R: FromStr,
    <R as FromStr>::Err: std::fmt::Display,
    R: Into<T>,
{
    let maybe = extract_single::<R>(name, extensions).ok()?;
    match maybe {
        Some(value) => Some(value.into()),
        None => extract_single::<R>(fallback, extensions)
            .ok()?
            .map(Into::into),
    }
}

pub(crate) fn extract_multiple<R>(
    name: &str,
    extensions: &mut BTreeMap<String, Vec<String>>,
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
        version: "1",
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
        supplemental_trace: None,
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
        version: "1",
        arrival_date: None,
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
X-KumoRef: eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoidGVzdEBleGFtcGxlLmNvbSJ9
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
        supplemental_trace: Some(
            Object {
                "recipient": String("test@example.com"),
            },
        ),
    },
)
"#
        );
    }

    #[test]
    fn rfc5965_3() {
        let result = ARFReport::parse(include_bytes!("../data/rfc5965/3.eml")).unwrap();
        k9::snapshot!(
            result,
            r#"
Some(
    ARFReport {
        feedback_type: "abuse",
        user_agent: "Yahoo!-Mail-Feedback/2.0",
        version: "0.1",
        arrival_date: Some(
            2023-12-14T16:16:15Z,
        ),
        incidents: None,
        original_envelope_id: None,
        original_mail_from: Some(
            "<test1@example.com>",
        ),
        reporting_mta: None,
        source_ip: None,
        authentication_results: [
            "authentication result string is not available",
        ],
        original_rcpto_to: [
            "user@example.com",
        ],
        reported_domain: [
            "bounce.kumo.example.com",
        ],
        reported_uri: [],
        extensions: {},
        original_message: Some(
            "Date: Thu, 14 Dec 2023 16:16:14 +0000
To: user@example.com
Subject: test Thu, 14 Dec 2023 16:16:14 +0000

This is a test mailing

",
        ),
        supplemental_trace: None,
    },
)
"#
        );
    }

    #[test]
    fn rfc5965_4() {
        let result = ARFReport::parse(include_bytes!("../data/rfc5965/4.eml")).unwrap();
        k9::snapshot!(
            result,
            r#"
Some(
    ARFReport {
        feedback_type: "abuse",
        user_agent: "ReturnPathFBL/2.0",
        version: "1",
        arrival_date: Some(
            2023-12-13T19:03:30Z,
        ),
        incidents: None,
        original_envelope_id: None,
        original_mail_from: Some(
            "foo@bounce.example.com",
        ),
        reporting_mta: None,
        source_ip: Some(
            "x.x.x.x",
        ),
        authentication_results: [],
        original_rcpto_to: [
            "cb4a01a48251d4765f489076aa81e2a4@comcast.net",
        ],
        reported_domain: [
            "bounce.example.com",
        ],
        reported_uri: [],
        extensions: {
            "abuse-type": [
                "complaint",
            ],
            "source": [
                "Comcast",
            ],
            "subscription-link": [
                "https://fbl.returnpath.net/manage/subscriptions/xxxx",
            ],
        },
        original_message: Some(
            "Date: Thu, 14 Dec 2023 16:16:14 +0000
To: user@example.com
Subject: test Thu, 14 Dec 2023 16:16:14 +0000

This is a test mailing

",
        ),
        supplemental_trace: None,
    },
)
"#
        );
    }
}
