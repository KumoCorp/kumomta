use crate::headermap::{EncodeHeaderValue, HeaderMap};
use crate::rfc5322_parser::Parser;
use crate::{AddressList, MailParsingError, Mailbox, MailboxList, Result, SharedString};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
    pub struct HeaderConformance: u8 {
        const MISSING_COLON_VALUE = 0b0000_0001;
        const NON_CANONICAL_LINE_ENDINGS = 0b0000_0010;
    }
}

#[derive(Clone, Debug)]
pub struct Header<'a> {
    /// The name portion of the header
    name: SharedString<'a>,
    /// The value portion of the header
    value: SharedString<'a>,
    /// The separator between the name and the value
    separator: SharedString<'a>,
    conformance: HeaderConformance,
}

/// Holds the result of parsing a block of headers
pub struct HeaderParseResult<'a> {
    pub headers: HeaderMap<'a>,
    pub body_offset: usize,
    pub overall_conformance: HeaderConformance,
}

impl<'a> Header<'a> {
    pub fn with_name_value<N: Into<SharedString<'a>>, V: Into<SharedString<'a>>>(
        name: N,
        value: V,
    ) -> Self {
        let name = name.into();
        let value = value.into();
        Self {
            name,
            value,
            separator: ": ".into(),
            conformance: HeaderConformance::default(),
        }
    }

    pub fn new<N: Into<SharedString<'a>>>(name: N, value: impl EncodeHeaderValue) -> Self {
        let name = name.into();
        let value = value.encode_value();
        Self {
            name,
            value,
            separator: ": ".into(),
            conformance: HeaderConformance::default(),
        }
    }

    pub fn assign(&mut self, v: impl EncodeHeaderValue) {
        self.value = v.encode_value();
    }

    /// Format the header into the provided output stream,
    /// as though writing it out as part of a mime part
    pub fn write_header<W: std::io::Write>(&self, mut out: W) -> std::io::Result<()> {
        let line_ending = if self
            .conformance
            .contains(HeaderConformance::NON_CANONICAL_LINE_ENDINGS)
        {
            "\n"
        } else {
            "\r\n"
        };
        write!(
            out,
            "{}{}{}{line_ending}",
            self.name, self.separator, self.value
        )
    }

    /// Convenience method wrapping write_header that returns
    /// the formatted header as a standalone string
    pub fn to_header_string(&self) -> String {
        let mut out = vec![];
        self.write_header(&mut out).unwrap();
        String::from_utf8_lossy(&out).to_string()
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_raw_value(&self) -> &str {
        &self.value
    }

    /// Parse the header into a mailbox-list (as defined in
    /// RFC 5322), which is how the `From` and `Resent-From`,
    /// headers are defined.
    pub fn as_mailbox_list(&self) -> Result<MailboxList> {
        Parser::parse_mailbox_list_header(self.get_raw_value())
    }

    /// Parse the header into a mailbox (as defined in
    /// RFC 5322), which is how the `Sender` and `Resent-Sender`
    /// headers are defined.
    pub fn as_mailbox(&self) -> Result<Mailbox> {
        Parser::parse_mailbox_header(self.get_raw_value())
    }

    pub fn as_address_list(&self) -> Result<AddressList> {
        Parser::parse_address_list_header(self.get_raw_value())
    }

    pub fn as_message_id(&self) -> Result<String> {
        Parser::parse_msg_id_header(self.get_raw_value())
    }

    pub fn as_message_id_list(&self) -> Result<Vec<String>> {
        Parser::parse_msg_id_header_list(self.get_raw_value())
    }

    pub fn as_unstructured(&self) -> Result<String> {
        Parser::parse_unstructured_header(self.get_raw_value())
    }

    pub fn parse_headers<S: Into<SharedString<'a>>>(
        header_block: S,
    ) -> Result<HeaderParseResult<'a>> {
        let header_block = header_block.into();
        let mut headers = vec![];
        let mut idx = 0;
        let mut overall_conformance = HeaderConformance::default();

        while idx < header_block.len() {
            let b = header_block[idx];
            if headers.is_empty() {
                if b.is_ascii_whitespace() {
                    return Err(MailParsingError::HeaderParse(
                        "header block must not start with spaces".to_string(),
                    ));
                }
            }
            if b == b'\n' {
                // LF: End of header block
                idx += 1;
                overall_conformance.set(HeaderConformance::NON_CANONICAL_LINE_ENDINGS, true);
                break;
            }
            if b == b'\r' {
                if idx + 1 < header_block.len() && header_block[idx + 1] == b'\n' {
                    // CRLF: End of header block
                    idx += 2;
                    break;
                }
                return Err(MailParsingError::HeaderParse(
                    "lone CR in header".to_string(),
                ));
            }
            let (header, next) = Self::parse(header_block.slice(idx..header_block.len()))?;
            overall_conformance |= header.conformance;
            headers.push(header);
            debug_assert!(
                idx != next + idx,
                "idx={idx}, next={next}, headers: {headers:#?}"
            );
            idx += next;
        }
        Ok(HeaderParseResult {
            headers: HeaderMap::new(headers),
            body_offset: idx,
            overall_conformance,
        })
    }

    pub fn parse<S: Into<SharedString<'a>>>(header_block: S) -> Result<(Self, usize)> {
        let header_block = header_block.into();

        enum State {
            Initial,
            Name,
            Separator,
            Value,
            NewLine,
        }

        let mut state = State::Initial;

        let mut iter = header_block.as_bytes().iter();
        let mut c = *iter
            .next()
            .ok_or_else(|| MailParsingError::HeaderParse("empty header string".to_string()))?;

        let mut name_end = None;
        let mut value_start = 0;
        let mut value_end = 0;

        let mut idx = 0;
        let mut conformance = HeaderConformance::default();
        let mut saw_cr = false;

        loop {
            match state {
                State::Initial => {
                    if c.is_ascii_whitespace() {
                        return Err(MailParsingError::HeaderParse(format!(
                            "header cannot start with space"
                        )));
                    }
                    state = State::Name;
                    continue;
                }
                State::Name => {
                    if c == b':' {
                        name_end.replace(idx);
                        state = State::Separator;
                    } else if c < 33 || c > 126 {
                        return Err(MailParsingError::HeaderParse(format!(
                            "header name must be comprised of printable US-ASCII characters. Found {c:?}"
                        )));
                    } else if c == b'\n' {
                        // Got a newline before we finished parsing the name
                        conformance.set(HeaderConformance::MISSING_COLON_VALUE, true);
                        name_end.replace(idx);
                        value_start = idx;
                        value_end = idx;
                        idx += 1;
                        break;
                    }
                }
                State::Separator => {
                    if c != b' ' {
                        value_start = idx;
                        value_end = idx;
                        state = State::Value;
                        continue;
                    }
                }
                State::Value => {
                    if c == b'\n' {
                        if !saw_cr {
                            conformance.set(HeaderConformance::NON_CANONICAL_LINE_ENDINGS, true);
                        }
                        state = State::NewLine;
                        saw_cr = false;
                    } else if c != b'\r' {
                        value_end = idx + 1;
                        saw_cr = false;
                    } else {
                        saw_cr = true;
                    }
                }
                State::NewLine => {
                    if c == b' ' || c == b'\t' {
                        state = State::Value;
                        continue;
                    }
                    break;
                }
            }
            idx += 1;
            c = match iter.next() {
                None => break,
                Some(v) => *v,
            };
        }

        let name_end = name_end.unwrap_or_else(|| {
            conformance.set(HeaderConformance::MISSING_COLON_VALUE, true);
            idx
        });

        let name = header_block.slice(0..name_end);
        let value = header_block.slice(value_start..value_end);
        let separator = header_block.slice(name_end..value_start);

        let header = Self {
            name,
            value,
            separator,
            conformance,
        };

        Ok((header, idx))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn assert_static_lifetime(_header: Header<'static>) {
        assert!(true, "I wouldn't compile if this wasn't true");
    }

    #[test]
    fn header_construction() {
        let header = Header::with_name_value("To", "someone@example.com");
        assert_eq!(header.get_name(), "To");
        assert_eq!(header.get_raw_value(), "someone@example.com");
        assert_eq!(header.to_header_string(), "To: someone@example.com\r\n");
        assert_static_lifetime(header);
    }

    #[test]
    fn header_parsing() {
        let message = concat!(
            "Subject: hello there\n",
            "From:  Someone <someone@example.com>\n",
            "\n",
            "I am the body"
        );

        let HeaderParseResult {
            headers,
            body_offset,
            overall_conformance,
        } = Header::parse_headers(message).unwrap();
        assert_eq!(&message[body_offset..], "I am the body");
        k9::snapshot!(
            overall_conformance,
            "
HeaderConformance(
    NON_CANONICAL_LINE_ENDINGS,
)
"
        );
        k9::snapshot!(
            headers,
            r#"
HeaderMap {
    headers: [
        Header {
            name: "Subject",
            value: "hello there",
            separator: ": ",
            conformance: HeaderConformance(
                NON_CANONICAL_LINE_ENDINGS,
            ),
        },
        Header {
            name: "From",
            value: "Someone <someone@example.com>",
            separator: ":  ",
            conformance: HeaderConformance(
                NON_CANONICAL_LINE_ENDINGS,
            ),
        },
    ],
}
"#
        );
    }

    #[test]
    fn assign_mailbox() {
        let mut sender = Header::with_name_value("Sender", "");
        sender.assign(Mailbox {
            name: Some("John Smith".to_string()),
            address: "john.smith@example.com".to_string(),
        });
        assert_eq!(
            sender.to_header_string(),
            "Sender: \"John Smith\" <john.smith@example.com>\r\n"
        );
    }

    #[test]
    fn new_mailbox() {
        let sender = Header::new(
            "Sender",
            Mailbox {
                name: Some("John".to_string()),
                address: "john.smith@example.com".to_string(),
            },
        );
        assert_eq!(
            sender.to_header_string(),
            "Sender: John <john.smith@example.com>\r\n"
        );
    }

    #[test]
    fn new_mailbox_2047() {
        let sender = Header::new(
            "Sender",
            Mailbox {
                name: Some("André Pirard".to_string()),
                address: "andre@example.com".to_string(),
            },
        );
        assert_eq!(
            sender.to_header_string(),
            "Sender: =?UTF-8?q?Andr=C3=A9_Pirard?= <andre@example.com>\r\n"
        );
    }
}
