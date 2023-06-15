use crate::{parser, DKIMError};
use indexmap::map::IndexMap;
use std::io::Write;
use std::str::FromStr;

pub(crate) const HEADER: &str = "DKIM-Signature";
const REQUIRED_TAGS: &[&str] = &["v", "a", "b", "bh", "d", "h", "s"];
const SIGN_EXPIRATION_DRIFT_MINS: i64 = 15;

#[derive(Debug, Clone)]
pub(crate) struct DKIMHeader {
    tags: IndexMap<String, parser::Tag>,
    pub raw_bytes: String,
}

impl DKIMHeader {
    /// <https://datatracker.ietf.org/doc/html/rfc6376#section-6.1.1>
    pub fn parse(value: &str) -> Result<Self, DKIMError> {
        let (_, tags) = parser::tag_list(value)
            .map_err(|err| DKIMError::SignatureSyntaxError(err.to_string()))?;

        let mut tags_map = IndexMap::new();
        for tag in &tags {
            tags_map.insert(tag.name.clone(), tag.clone());
        }
        let header = DKIMHeader {
            tags: tags_map,
            raw_bytes: value.to_owned(),
        };

        header.validate_required_tags()?;

        // Check version
        {
            let version = header.get_required_tag("v");
            if version != "1" {
                return Err(DKIMError::IncompatibleVersion);
            }
        }

        // Check that "d=" tag is the same as or a parent domain of the domain part
        // of the "i=" tag
        if let Some(user) = header.get_tag("i") {
            let signing_domain = header.get_required_tag("d");
            // TODO: naive check, should switch to parsing the domains/email
            if !user.ends_with(&signing_domain) {
                return Err(DKIMError::DomainMismatch);
            }
        }

        // Check that "h=" tag includes the From header
        {
            let value = header.get_required_tag("h");
            let headers = value.split(':');
            let headers: Vec<String> = headers.map(|h| h.to_lowercase()).collect();
            if !headers.contains(&"from".to_string()) {
                return Err(DKIMError::FromFieldNotSigned);
            }
        }

        if let Some(query_method) = header.get_tag("q") {
            if query_method != "dns/txt" {
                return Err(DKIMError::UnsupportedQueryMethod);
            }
        }

        // Check that "x=" tag isn't expired
        if let Some(expiration) = header.get_tag("x") {
            let mut expiration = chrono::NaiveDateTime::from_timestamp_opt(
                expiration.parse::<i64>().unwrap_or_default(),
                0,
            )
            .ok_or(DKIMError::SignatureExpired)?;
            expiration += chrono::Duration::minutes(SIGN_EXPIRATION_DRIFT_MINS);
            let now = chrono::Utc::now().naive_utc();
            if now > expiration {
                return Err(DKIMError::SignatureExpired);
            }
        }

        Ok(header)
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

    fn validate_required_tags(&self) -> Result<(), DKIMError> {
        for required in REQUIRED_TAGS {
            if self.get_tag(required).is_none() {
                return Err(DKIMError::SignatureMissingRequiredTag(required));
            }
        }
        Ok(())
    }
}

/// Generate the DKIM-Signature header from the tags
fn serialize(header: DKIMHeader) -> Result<String, DKIMError> {
    let mut out = vec![];

    for (key, tag) in &header.tags {
        if !out.is_empty() {
            out.push(b' ');
        }
        write!(&mut out, "{}={};", key, tag.value)
            .map_err(|err| DKIMError::HeaderSerializeError(format!("while appending: {err:#}")))?;
    }

    String::from_utf8(out)
        .map_err(|err| DKIMError::HeaderSerializeError(format!("converting to string: {err:#}")))
}

#[derive(Clone)]
pub(crate) struct DKIMHeaderBuilder {
    header: DKIMHeader,
    time: Option<chrono::DateTime<chrono::offset::Utc>>,
}
impl DKIMHeaderBuilder {
    pub(crate) fn new() -> Self {
        DKIMHeaderBuilder {
            header: DKIMHeader {
                tags: IndexMap::new(),
                raw_bytes: "".to_owned(),
            },
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

    pub(crate) fn set_signed_headers(self, headers: &Vec<String>) -> Self {
        let value = headers.join(":");
        self.add_tag("h", &value)
    }

    pub(crate) fn set_expiry(self, duration: chrono::Duration) -> Result<Self, DKIMError> {
        let time = self
            .time
            .ok_or(DKIMError::BuilderError("missing require time"))?;
        let expiry = (time + duration).timestamp();
        Ok(self.add_tag("x", &expiry.to_string()))
    }

    pub(crate) fn set_time(mut self, time: chrono::DateTime<chrono::offset::Utc>) -> Self {
        self.time = Some(time);
        self.add_tag("t", &time.timestamp().to_string())
    }

    pub(crate) fn build(mut self) -> Result<DKIMHeader, DKIMError> {
        self.header.raw_bytes = serialize(self.header.clone())?;
        Ok(self.header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dkim_header_builder() {
        let header = DKIMHeaderBuilder::new()
            .add_tag("v", "1")
            .add_tag("a", "something")
            .build()
            .unwrap();
        assert_eq!(header.raw_bytes, "v=1; a=something;".to_owned());
    }

    fn signed_header_list(headers: &[&str]) -> Vec<String> {
        headers.into_iter().map(|h| h.to_lowercase()).collect()
    }

    #[test]
    fn test_dkim_header_builder_signed_headers() {
        let header = DKIMHeaderBuilder::new()
            .add_tag("v", "2")
            .set_signed_headers(&signed_header_list(&["header1", "header2", "header3"]))
            .build()
            .unwrap();
        assert_eq!(
            header.raw_bytes,
            "v=2; h=header1:header2:header3;".to_owned()
        );
    }

    #[test]
    fn test_dkim_header_builder_time() {
        use chrono::TimeZone;

        let time = chrono::Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 1).unwrap();

        let header = DKIMHeaderBuilder::new()
            .set_time(time)
            .set_expiry(chrono::Duration::hours(3))
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(header.raw_bytes, "t=1609459201; x=1609470001;".to_owned());
    }
}
