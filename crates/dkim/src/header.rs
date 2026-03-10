use crate::arc::{ARC_SEAL_HEADER_NAME, MAX_ARC_INSTANCE};
use crate::{parser, DKIMError, HeaderList};
use dns_resolver::{Name, Resolver};
use indexmap::map::IndexMap;
use mailparsing::Header;
use std::str::FromStr;

pub(crate) const DKIM_SIGNATURE_HEADER_NAME: &str = "DKIM-Signature";
const SIGN_EXPIRATION_DRIFT_MINS: i64 = 15;

#[derive(Clone, Debug, Default)]
pub struct TaggedHeader {
    tags: IndexMap<String, parser::Tag>,
    raw_bytes: String,
}

impl TaggedHeader {
    pub fn parse(value: &str) -> Result<Self, DKIMError> {
        let (_, tags) = parser::tag_list(value)
            .map_err(|err| DKIMError::SignatureSyntaxError(err.to_string()))?;

        let mut tags_map = IndexMap::new();
        for tag in &tags {
            tags_map.insert(tag.name.clone(), tag.clone());
        }
        Ok(Self {
            tags: tags_map,
            raw_bytes: value.to_owned(),
        })
    }

    pub fn get_tag(&self, name: &str) -> Option<&str> {
        self.tags.get(name).map(|v| v.value.as_str())
    }

    /// Get the named tag.
    /// Attempt to parse it into an `R`
    pub fn parse_tag<R>(&self, name: &str) -> Result<Option<R>, DKIMError>
    where
        R: FromStr,
        <R as FromStr>::Err: std::fmt::Display,
    {
        match self.get_tag(name) {
            None => Ok(None),
            Some(value) => {
                let value: R = value.parse().map_err(|err| {
                    DKIMError::SignatureSyntaxError(format!(
                        "invalid \"{name}\" tag value: {err:#}"
                    ))
                })?;
                Ok(Some(value))
            }
        }
    }

    pub fn get_raw_tag(&self, name: &str) -> Option<&str> {
        self.tags.get(name).map(|v| v.raw_value.as_str())
    }

    pub fn get_required_tag(&self, name: &str) -> &str {
        // Required tags are guaranteed by the parser to be present so it's safe
        // to assert and unwrap.
        match self.get_tag(name) {
            Some(value) => value,
            None => panic!("required tag {name} is not present"),
        }
    }

    pub fn get_required_raw_tag(&self, name: &str) -> &str {
        // Required tags are guaranteed by the parser to be present so it's safe
        // to assert and unwrap.
        match self.get_raw_tag(name) {
            Some(value) => value,
            None => panic!("required tag {name} is not present"),
        }
    }

    pub fn raw(&self) -> &str {
        &self.raw_bytes
    }

    pub fn arc_instance(&self) -> Result<u8, DKIMError> {
        let instance = self
            .get_required_tag("i")
            .parse::<u8>()
            .map_err(|_| DKIMError::InvalidARCInstance)?;

        if instance == 0 || instance > MAX_ARC_INSTANCE {
            return Err(DKIMError::InvalidARCInstance);
        }

        Ok(instance)
    }

    /// Generate the DKIM-Signature header from the tags
    fn serialize(&self) -> String {
        let mut lines = vec![];
        let mut line = String::new();

        const WIDTH: usize = 75;

        for (key, tag) in &self.tags {
            let value = &tag.value;

            // Always emit b on a separate line for the sake of
            // consistency of the hash, which is generated in two
            // passes; the first with an empty b value and the second
            // with it populated.
            // If we don't push it to the next line, the two passes
            // may produce inconsistent results as a result of the
            // text wrapping and the signature will be invalid.
            //
            // Similarly, header lists can be rather long and we
            // want to control how they wrap with a bit more nuance.
            let always_new_line = key == "b" || key == "h";

            if always_new_line || (line.len() + key.len() + 2 + value.len() >= WIDTH) {
                if !line.is_empty() {
                    lines.push(line.clone());
                    line.clear();
                }
            }

            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(key);
            line.push('=');

            if line.len() + value.len() < WIDTH {
                line.push_str(value);
            } else if key == "h" {
                for (idx, name) in value.split(':').enumerate() {
                    if idx > 0 {
                        line.push(':');
                    }
                    if line.len() + name.len() < WIDTH {
                        line.push_str(name);
                        continue;
                    }

                    // Need new line
                    lines.push(line);
                    line = format!("\t{name}");
                }
            } else {
                if value.len() >= WIDTH {
                    // Value will never fit even on a fresh line,
                    // so we force it to break
                    for c in value.chars() {
                        line.push(c);
                        if line.len() >= WIDTH {
                            lines.push(line.clone());
                            line.clear();
                        }
                    }
                } else {
                    lines.push(line);
                    line = format!("\t{value}");
                }
            }
            line.push(';');
        }

        if !line.is_empty() {
            lines.push(line);
        }

        lines.join("\r\n\t")
    }

    /// Check things common to DKIM-Signature and ARC-Message-Signature
    fn check_common_tags(&self) -> Result<(), DKIMError> {
        // Check that "h=" tag includes the From header
        if !self
            .get_required_tag("h")
            .split(':')
            .any(|h| h.eq_ignore_ascii_case("from"))
        {
            return Err(DKIMError::FromFieldNotSigned);
        }

        if let Some(query_method) = self.get_tag("q") {
            if query_method != "dns/txt" {
                return Err(DKIMError::UnsupportedQueryMethod);
            }
        }

        // Check that "x=" tag isn't expired
        if let Some(expiration) = self.get_tag("x") {
            let mut expiration =
                chrono::DateTime::from_timestamp(expiration.parse::<i64>().unwrap_or_default(), 0)
                    .ok_or(DKIMError::SignatureExpired)?;
            expiration += chrono::Duration::try_minutes(SIGN_EXPIRATION_DRIFT_MINS)
                .expect("drift to be in-range");
            let now = chrono::Utc::now();
            if now > expiration {
                return Err(DKIMError::SignatureExpired);
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DKIMHeader {
    tagged: TaggedHeader,
}

impl std::ops::Deref for DKIMHeader {
    type Target = TaggedHeader;
    fn deref(&self) -> &TaggedHeader {
        &self.tagged
    }
}
impl std::ops::DerefMut for DKIMHeader {
    fn deref_mut(&mut self) -> &mut TaggedHeader {
        &mut self.tagged
    }
}

impl DKIMHeader {
    /// <https://datatracker.ietf.org/doc/html/rfc6376#section-6.1.1>
    pub fn parse(value: &str) -> Result<Self, DKIMError> {
        let tagged = TaggedHeader::parse(value)?;
        let header = DKIMHeader { tagged };

        header.validate_required_tags()?;

        // Check version
        if header.get_required_tag("v") != "1" {
            return Err(DKIMError::IncompatibleVersion);
        }

        // Check that "d=" tag is the same as or a parent domain of the domain part
        // of the "i=" tag
        if let Some(user) = header.get_tag("i") {
            let signing_domain = header.get_required_tag("d");
            let Some((_local, domain)) = user.split_once('@') else {
                return Err(DKIMError::DomainMismatch);
            };

            let i_domain = Name::from_str_relaxed(domain).map_err(|_| DKIMError::DomainMismatch)?;
            let d_domain =
                Name::from_str_relaxed(signing_domain).map_err(|_| DKIMError::DomainMismatch)?;

            if !d_domain.zone_of(&i_domain) {
                return Err(DKIMError::DomainMismatch);
            }
        }

        header.check_common_tags()?;

        Ok(header)
    }

    fn validate_required_tags(&self) -> Result<(), DKIMError> {
        const REQUIRED_TAGS: &[&str] = &["v", "a", "b", "bh", "d", "h", "s"];
        for required in REQUIRED_TAGS {
            if self.get_tag(required).is_none() {
                return Err(DKIMError::SignatureMissingRequiredTag(required));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct TaggedHeaderBuilder {
    header: TaggedHeader,
    time: Option<chrono::DateTime<chrono::offset::Utc>>,
}
impl TaggedHeaderBuilder {
    pub(crate) fn new() -> Self {
        TaggedHeaderBuilder {
            header: TaggedHeader::default(),
            time: None,
        }
    }

    pub(crate) fn add_tag(mut self, name: &str, value: &str) -> Self {
        let tag = parser::Tag {
            name: name.to_owned(),
            value: value.to_owned(),
            raw_value: value.to_owned(),
        };
        self.header.tags.insert(name.to_owned(), tag);

        self
    }

    pub(crate) fn set_signed_headers(self, headers: &HeaderList) -> Self {
        let value = headers.as_h_list();
        self.add_tag("h", &value)
    }

    pub(crate) fn set_expiry(self, duration: chrono::Duration) -> Result<Self, DKIMError> {
        let time = self.time.ok_or(DKIMError::BuilderError(
            "TaggedHeaderBuilder: set_time must be called prior to calling set_expiry",
        ))?;
        let expiry = (time + duration).timestamp();
        Ok(self.add_tag("x", &expiry.to_string()))
    }

    pub(crate) fn set_time(mut self, time: chrono::DateTime<chrono::offset::Utc>) -> Self {
        self.time = Some(time);
        self.add_tag("t", &time.timestamp().to_string())
    }

    pub(crate) fn build(mut self) -> TaggedHeader {
        self.header.raw_bytes = self.header.serialize();
        self.header
    }
}

/// <https://datatracker.ietf.org/doc/html/rfc8617#section-4.1.2> says
/// The AMS header field has the same syntax and semantics as the
/// DKIM-Signature field [RFC6376], with three (3) differences
/// * the name of the header field itself;
/// * no version tag ("v") is defined for the AMS header field.
///   As required for undefined tags (in
///   [RFC6376]), if seen, a version tag MUST be ignored.
/// * the "i" (Agent or User Identifier (AUID)) tag is not imported from
///   DKIM; instead, this tag is replaced by the instance tag as defined
///   in Section 4.2.1.
#[derive(Debug, Clone, Default)]
pub struct ARCMessageSignatureHeader {
    tagged: TaggedHeader,
}

impl std::ops::Deref for ARCMessageSignatureHeader {
    type Target = TaggedHeader;
    fn deref(&self) -> &TaggedHeader {
        &self.tagged
    }
}
impl std::ops::DerefMut for ARCMessageSignatureHeader {
    fn deref_mut(&mut self) -> &mut TaggedHeader {
        &mut self.tagged
    }
}

impl ARCMessageSignatureHeader {
    pub fn parse(value: &str) -> Result<Self, DKIMError> {
        let tagged = TaggedHeader::parse(value)?;
        let header = Self { tagged };

        header.validate_required_tags()?;
        header.check_common_tags()?;
        header.arc_instance()?;

        Ok(header)
    }

    fn validate_required_tags(&self) -> Result<(), DKIMError> {
        const REQUIRED_TAGS: &[&str] = &["a", "b", "bh", "d", "h", "s", "i"];
        for required in REQUIRED_TAGS {
            if self.get_tag(required).is_none() {
                return Err(DKIMError::SignatureMissingRequiredTag(required));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ARCSealHeader {
    tagged: TaggedHeader,
}

impl std::ops::Deref for ARCSealHeader {
    type Target = TaggedHeader;
    fn deref(&self) -> &TaggedHeader {
        &self.tagged
    }
}
impl std::ops::DerefMut for ARCSealHeader {
    fn deref_mut(&mut self) -> &mut TaggedHeader {
        &mut self.tagged
    }
}

impl ARCSealHeader {
    pub fn parse(value: &str) -> Result<Self, DKIMError> {
        let tagged = TaggedHeader::parse(value)?;
        let header = Self { tagged };

        header.validate_required_tags()?;
        header.arc_instance()?;

        if header.get_tag("h").is_some() {
            // TODO: MUST result in cv status of fail, see Section 5.1.1
        }

        Ok(header)
    }

    fn validate_required_tags(&self) -> Result<(), DKIMError> {
        const REQUIRED_TAGS: &[&str] = &["a", "b", "d", "s", "i", "cv"];
        for required in REQUIRED_TAGS {
            if self.get_tag(required).is_none() {
                return Err(DKIMError::SignatureMissingRequiredTag(required));
            }
        }
        Ok(())
    }

    pub async fn verify(
        &self,
        resolver: &dyn Resolver,
        header_list: &Vec<&Header<'_>>,
    ) -> Result<(), DKIMError> {
        let public_keys = crate::public_key::retrieve_public_keys(
            resolver,
            self.get_required_tag("d"),
            self.get_required_tag("s"),
        )
        .await?;

        let hash_algo = parser::parse_hash_algo(self.get_required_tag("a"))?;

        let computed_headers_hash = crate::hash::compute_headers_hash(
            crate::canonicalization::Type::Relaxed,
            &header_list,
            hash_algo,
            self,
            ARC_SEAL_HEADER_NAME,
        )?;

        let signature = data_encoding::BASE64
            .decode(self.get_required_tag("b").as_bytes())
            .map_err(|err| {
                DKIMError::SignatureSyntaxError(format!("failed to decode signature: {}", err))
            })?;

        let mut errors = vec![];
        for public_key in public_keys {
            match crate::verify_signature(hash_algo, &computed_headers_hash, &signature, public_key)
            {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(err) => {
                    errors.push(err);
                }
            }
        }

        if let Some(err) = errors.pop() {
            // Something definitely failed
            return Err(err);
        }

        // There were no errors and all keys returned false from verify_signature().
        // That means that the signature is not verified.
        Err(DKIMError::SignatureDidNotVerify)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dkim_header_builder() {
        let header = TaggedHeaderBuilder::new()
            .add_tag("v", "1")
            .add_tag("a", "something")
            .build();
        k9::snapshot!(header.raw(), "v=1; a=something;");
    }

    fn signed_header_list(headers: &[&str]) -> HeaderList {
        HeaderList::new(headers.iter().map(|h| h.to_lowercase()).collect())
    }

    #[test]
    fn test_dkim_header_builder_signed_headers() {
        let header = TaggedHeaderBuilder::new()
            .add_tag("v", "2")
            .set_signed_headers(&signed_header_list(&["header1", "header2", "header3"]))
            .build();
        k9::snapshot!(
            header.raw(),
            r#"
v=2;\r
\th=header1:header2:header3;
"#
        );
    }

    #[test]
    fn test_dkim_header_builder_time() {
        use chrono::TimeZone;

        let time = chrono::Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 1).unwrap();

        let header = TaggedHeaderBuilder::new()
            .set_time(time)
            .set_expiry(chrono::Duration::try_hours(3).expect("3 hours ok"))
            .unwrap()
            .build();
        k9::snapshot!(header.raw(), "t=1609459201; x=1609470001;");
    }

    #[test]
    fn test_parse_ams() {
        let sig = "i=1; a=rsa-sha256; c=relaxed/relaxed; d=
    messagingengine.com; h=date:from:reply-to:to:message-id:subject
    :mime-version:content-type:content-transfer-encoding; s=fm3; t=
    1761717439; bh=+BM/Umiva3F0xjsh9a2BcwzO1nr0Ru6oGRmgkMy9T3M=; b=I
    M7xjn2qSjOx5fDFvQY+pEPJ74+w3h/UOZUKvdAt7gRP8rAe9C+Tz72izVJyY82xw
    7LT7CBXnwk2DQpg9erhq1yYept4M5CKWLXoQHHUJam8mV4RMUnHgTLVlColIVUtY
    hNAomZdsGNiG1iRGX0C4y81zYANJ11TXKOTvfuMLhG2uDIa8768O5jBa4jlBtGHd
    Dn/87/T/J+plO/ZPiSwWKa+ZttR6yjwm0fdpXf+4y8u0+I8iYSw2EN0vgWMYEEMp
    R1xuhMKD+bSlx130Rz2/5jFsVgLS7CfbTKK5CtqS3hl6EaLw/REBZeCYCHltzRWF
    wt38/NIzJ3ykCswwds2YQ==";
        ARCMessageSignatureHeader::parse(sig).unwrap();
    }

    #[test]
    fn test_parse_as() {
        let seal = "i=1; a=rsa-sha256; cv=none; d=messagingengine.com; s=fm3; t=
    1761717439; b=Q1E9HuR4H0paxIiz15H8P3tGfzDp0XmYKhvyzGsPEBHr2xg610
    ZV1nU6gLWmUl693usMKVxWGrIXbSZb13ICRK0gp1MfVJSQ/4IGM0VD9P5d9Vv7aL
    Q/lx/a8Ar1ks1yEHeBRuZ6Q5GdYur8rgYr7UoOTJGwOOPTJ4C2TWGoHHIRoVECJv
    mMa6jpcJ6SE6iK/76elugk65BheumbQ1YEnbjitchUsLAwSXMuO+mhLYGtmvBhOn
    v3ewYQvD2jZzl2W+O73A08dQ/oeODDPqt6Fpv3XK572cTYPHhzmSbsxh9Lp7Z9MV
    x2TACmO51Adnp3C1CcEw8K9ajAgyjNMW4ELA==";
        ARCSealHeader::parse(seal).unwrap();
    }

    /// Check that the parsed values in each tag are the same.
    /// We don't compare the Tags directly as the raw values may
    /// have whitespace that differs
    fn check_tagged_header_equality(a: &TaggedHeader, b: &TaggedHeader) {
        use std::collections::HashMap;
        let a: HashMap<String, String> = a
            .tags
            .values()
            .map(|t| (t.name.clone(), t.value.clone()))
            .collect();
        let b: HashMap<String, String> = b
            .tags
            .values()
            .map(|t| (t.name.clone(), t.value.clone()))
            .collect();
        k9::assert_equal!(a, b);
    }

    fn make_tagged_header(domain: impl Into<String>) -> TaggedHeader {
        let headers = vec![
            "from",
            "to",
            "message-id",
            "date",
            "subject",
            "content-type",
            "mime-version",
            "list-unsubscribe",
            "list-unsubscribe-post",
        ];

        let domain = domain.into();

        TaggedHeaderBuilder::new()
            .add_tag("v", "1")
            .add_tag("a", "rsa-sha256")
            .add_tag("d", &domain)
            .add_tag("s", "stage")
            .add_tag("c", "relaxed/relaxed")
            .set_signed_headers(&signed_header_list(&headers))
            .add_tag("bh", "ecGWgWCJeWxJFeM0urOVWP+KOlqqvsQYKOpYUP8nk7I=")
            .add_tag("b", "abc123def456xyz789==")
            .build()
    }

    /// This is a regression test for a wrapping issue where our
    /// own header-list aware wrapping was in conflict with a second-pass
    /// generic textwrap::fill operation that could break our carefully
    /// wrapped header.
    /// The code that triggered that condition literally no longer exists
    /// in the code, but was triggered by this very specific combination
    /// of tag values so we're keeping it as a regression test.
    /// <https://github.com/KumoCorp/kumomta/pull/483> is a link to
    /// a PR describing the issue in more detail.
    #[test]
    fn test_long_header_list_with_wrapping() {
        let header = make_tagged_header("adobe-campaign.com");

        let raw = &header.raw_bytes;
        k9::snapshot!(
            &raw,
            r#"
v=1; a=rsa-sha256; d=adobe-campaign.com; s=stage; c=relaxed/relaxed;\r
\th=from:to:message-id:date:subject:content-type:mime-version:\r
\t\tlist-unsubscribe:list-unsubscribe-post;\r
\tbh=ecGWgWCJeWxJFeM0urOVWP+KOlqqvsQYKOpYUP8nk7I=;\r
\tb=abc123def456xyz789==;
"#
        );

        let round_trip = TaggedHeader::parse(raw).unwrap();
        check_tagged_header_equality(&header, &round_trip);
    }

    #[test]
    fn test_wrapping_2() {
        let header = make_tagged_header(format!("{}.com", "a".repeat(76)));

        let raw = &header.raw_bytes;
        k9::snapshot!(
            &raw,
            r#"
v=1; a=rsa-sha256;\r
\td=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r
\taaa.com; s=stage; c=relaxed/relaxed;\r
\th=from:to:message-id:date:subject:content-type:mime-version:\r
\t\tlist-unsubscribe:list-unsubscribe-post;\r
\tbh=ecGWgWCJeWxJFeM0urOVWP+KOlqqvsQYKOpYUP8nk7I=;\r
\tb=abc123def456xyz789==;
"#
        );

        let round_trip = TaggedHeader::parse(raw).unwrap();
        check_tagged_header_equality(&header, &round_trip);
    }

    #[test]
    fn test_wrapping_3() {
        let header = make_tagged_header(format!("{}.com", "a".repeat(50)));

        let raw = &header.raw_bytes;
        k9::snapshot!(
            &raw,
            r#"
v=1; a=rsa-sha256;\r
\td=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.com; s=stage;\r
\tc=relaxed/relaxed;\r
\th=from:to:message-id:date:subject:content-type:mime-version:\r
\t\tlist-unsubscribe:list-unsubscribe-post;\r
\tbh=ecGWgWCJeWxJFeM0urOVWP+KOlqqvsQYKOpYUP8nk7I=;\r
\tb=abc123def456xyz789==;
"#
        );

        let round_trip = TaggedHeader::parse(raw).unwrap();
        check_tagged_header_equality(&header, &round_trip);
    }

    #[test]
    fn test_wrapping_4() {
        let header = make_tagged_header(format!("{}.com", "a".repeat(49)));

        let raw = &header.raw_bytes;
        k9::snapshot!(
            &raw,
            r#"
v=1; a=rsa-sha256; d=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.com;\r
\ts=stage; c=relaxed/relaxed;\r
\th=from:to:message-id:date:subject:content-type:mime-version:\r
\t\tlist-unsubscribe:list-unsubscribe-post;\r
\tbh=ecGWgWCJeWxJFeM0urOVWP+KOlqqvsQYKOpYUP8nk7I=;\r
\tb=abc123def456xyz789==;
"#
        );

        let round_trip = TaggedHeader::parse(raw).unwrap();
        check_tagged_header_equality(&header, &round_trip);
    }

    #[test]
    fn test_wrapping_5() {
        let header = make_tagged_header(format!("{}.com", "a".repeat(70)));

        let raw = &header.raw_bytes;
        k9::snapshot!(
            &raw,
            r#"
v=1; a=rsa-sha256;\r
\td=\r
\t\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.com;\r
\ts=stage; c=relaxed/relaxed;\r
\th=from:to:message-id:date:subject:content-type:mime-version:\r
\t\tlist-unsubscribe:list-unsubscribe-post;\r
\tbh=ecGWgWCJeWxJFeM0urOVWP+KOlqqvsQYKOpYUP8nk7I=;\r
\tb=abc123def456xyz789==;
"#
        );

        let round_trip = TaggedHeader::parse(raw).unwrap();
        check_tagged_header_equality(&header, &round_trip);
    }
}
