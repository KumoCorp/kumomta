use crate::header::{HeaderConformance, HeaderParseResult};
use crate::{Header, Result, SharedString};

pub struct MimePart<'a> {
    /// The bytes that comprise this part, from its beginning to its end
    bytes: SharedString<'a>,
    /// The parsed headers from the start of bytes
    headers: Vec<Header<'a>>,
    /// The index into bytes of the first non-header byte.
    body_offset: usize,
    overall_conformance: HeaderConformance,
}

impl<'a> MimePart<'a> {
    pub fn parse<S: Into<SharedString<'a>>>(bytes: S) -> Result<Self> {
        let bytes = bytes.into();
        let HeaderParseResult {
            headers,
            body_offset,
            overall_conformance,
        } = Header::parse_headers(bytes.clone())?;
        Ok(Self {
            bytes,
            headers,
            body_offset,
            overall_conformance,
        })
    }

    pub fn headers(&self) -> &Vec<Header> {
        &self.headers
    }

    pub fn raw_body(&self) -> SharedString {
        self.bytes.slice(self.body_offset..self.bytes.len())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn msg_parsing() {
        let message = concat!(
            "Subject: hello there\n",
            "From:  Someone <someone@example.com>\n",
            "\n",
            "I am the body"
        );

        let part = MimePart::parse(message).unwrap();
        assert_eq!(part.raw_body(), "I am the body");
    }
}
