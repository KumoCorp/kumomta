use crate::{parser, DKIMError};
use indexmap::map::IndexMap;
use std::io::Write;

pub(crate) const HEADER: &str = "DKIM-Signature";
pub(crate) const REQUIRED_TAGS: &[&str] = &["v", "a", "b", "bh", "d", "h", "s"];

#[derive(Debug, Clone)]
pub(crate) struct DKIMHeader {
    pub tags: IndexMap<String, parser::Tag>,
    pub raw_bytes: String,
}

impl DKIMHeader {
    pub fn get_tag(&self, name: &str) -> Option<&str> {
        self.tags.get(name).map(|v| v.value.as_str())
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
