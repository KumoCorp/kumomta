use crate::headermap::{EncodeHeaderValue, HeaderMap};
use crate::rfc5322_parser::Parser;
use crate::{
    AddressList, AuthenticationResults, MailParsingError, Mailbox, MailboxList, MessageID,
    MimeParameters, Result, SharedString,
};
use chrono::{DateTime, FixedOffset};
use std::convert::TryInto;
use std::str::FromStr;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
    pub struct MessageConformance: u8 {
        const MISSING_COLON_VALUE = 0b0000_0001;
        const NON_CANONICAL_LINE_ENDINGS = 0b0000_0010;
        const NAME_ENDS_WITH_SPACE = 0b0000_0100;
        const LINE_TOO_LONG = 0b0000_1000;
        const NEEDS_TRANSFER_ENCODING = 0b0001_0000;
        const MISSING_DATE_HEADER = 0b0010_0000;
        const MISSING_MESSAGE_ID_HEADER = 0b0100_0000;
        const MISSING_MIME_VERSION = 0b1000_0000;
    }
}

impl FromStr for MessageConformance {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let mut result = Self::default();
        for ele in s.split('|') {
            if ele.is_empty() {
                continue;
            }
            match Self::from_name(ele) {
                Some(v) => {
                    result = result.union(v);
                }
                None => {
                    let mut possible: Vec<String> = Self::all()
                        .iter_names()
                        .map(|(name, _)| format!("'{name}'"))
                        .collect();
                    possible.sort();
                    let possible = possible.join(", ");
                    return Err(format!(
                        "invalid MessageConformance flag '{ele}', possible values are {possible}"
                    ));
                }
            }
        }
        Ok(result)
    }
}

impl ToString for MessageConformance {
    fn to_string(&self) -> String {
        let mut names: Vec<&str> = self.iter_names().map(|(name, _)| name).collect();
        names.sort();
        names.join("|")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Header<'a> {
    /// The name portion of the header
    name: SharedString<'a>,
    /// The value portion of the header
    value: SharedString<'a>,
    /// The separator between the name and the value
    separator: SharedString<'a>,
    conformance: MessageConformance,
}

/// Holds the result of parsing a block of headers
pub struct HeaderParseResult<'a> {
    pub headers: HeaderMap<'a>,
    pub body_offset: usize,
    pub overall_conformance: MessageConformance,
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
            conformance: MessageConformance::default(),
        }
    }

    pub fn new<N: Into<SharedString<'a>>>(name: N, value: impl EncodeHeaderValue) -> Self {
        let name = name.into();
        let value = value.encode_value();
        Self {
            name,
            value,
            separator: ": ".into(),
            conformance: MessageConformance::default(),
        }
    }

    pub fn new_unstructured<N: Into<SharedString<'a>>, V: Into<SharedString<'a>>>(
        name: N,
        value: V,
    ) -> Self {
        let name = name.into();
        let value = value.into();

        let value = if value.chars().all(|c| c.is_ascii()) {
            textwrap::fill(
                &value,
                textwrap::Options::new(75)
                    .initial_indent("")
                    .line_ending(textwrap::LineEnding::CRLF)
                    .word_separator(textwrap::WordSeparator::AsciiSpace)
                    .subsequent_indent("\t"),
            )
        } else {
            let mut encoded = String::with_capacity(value.len());
            let mut line_length = 0;
            let max_length = 75;
            for word in value.split_ascii_whitespace() {
                let quoted_word;
                let word = if word.is_ascii() {
                    word
                } else {
                    quoted_word = crate::rfc5322_parser::qp_encode(word);
                    &quoted_word
                };

                if line_length > 0 {
                    if word.len() < max_length - line_length {
                        encoded.push(' ');
                    } else {
                        encoded.push_str("\r\n\t");
                        line_length = 0;
                    }
                }
                encoded.push_str(word);
                line_length += word.len();
            }
            encoded
        }
        .into();

        Self {
            name,
            value,
            separator: ": ".into(),
            conformance: MessageConformance::default(),
        }
    }

    pub fn assign(&mut self, v: impl EncodeHeaderValue) {
        self.value = v.encode_value();
    }

    /// Format the header into the provided output stream,
    /// as though writing it out as part of a mime part
    pub fn write_header<W: std::io::Write>(&self, out: &mut W) -> std::io::Result<()> {
        let line_ending = if self
            .conformance
            .contains(MessageConformance::NON_CANONICAL_LINE_ENDINGS)
        {
            "\n"
        } else {
            "\r\n"
        };
        out.write_all(self.name.as_bytes())?;
        out.write_all(self.separator.as_bytes())?;
        out.write_all(self.value.as_bytes())?;
        out.write_all(line_ending.as_bytes())
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

    pub fn as_content_transfer_encoding(&self) -> Result<MimeParameters> {
        Parser::parse_content_transfer_encoding_header(self.get_raw_value())
    }

    pub fn as_content_disposition(&self) -> Result<MimeParameters> {
        Parser::parse_content_transfer_encoding_header(self.get_raw_value())
    }

    pub fn as_content_type(&self) -> Result<MimeParameters> {
        Parser::parse_content_type_header(self.get_raw_value())
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

    pub fn as_message_id(&self) -> Result<MessageID> {
        Parser::parse_msg_id_header(self.get_raw_value())
    }

    pub fn as_content_id(&self) -> Result<MessageID> {
        Parser::parse_content_id_header(self.get_raw_value())
    }

    pub fn as_message_id_list(&self) -> Result<Vec<MessageID>> {
        Parser::parse_msg_id_header_list(self.get_raw_value())
    }

    pub fn as_unstructured(&self) -> Result<String> {
        Parser::parse_unstructured_header(self.get_raw_value())
    }

    pub fn as_authentication_results(&self) -> Result<AuthenticationResults> {
        Parser::parse_authentication_results_header(self.get_raw_value())
    }

    pub fn as_date(&self) -> Result<DateTime<FixedOffset>> {
        DateTime::parse_from_rfc2822(self.get_raw_value()).map_err(MailParsingError::ChronoError)
    }

    pub fn parse_headers<S: TryInto<SharedString<'a>>>(
        header_block: S,
    ) -> Result<HeaderParseResult<'a>> {
        let header_block = header_block
            .try_into()
            .map_err(|_| MailParsingError::NotAscii)?;
        let mut headers = vec![];
        let mut idx = 0;
        let mut overall_conformance = MessageConformance::default();

        while idx < header_block.len() {
            let b = header_block[idx];
            if b == b'\n' {
                // LF: End of header block
                idx += 1;
                overall_conformance.set(MessageConformance::NON_CANONICAL_LINE_ENDINGS, true);
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
            if headers.is_empty() {
                if b.is_ascii_whitespace() {
                    return Err(MailParsingError::HeaderParse(
                        "header block must not start with spaces".to_string(),
                    ));
                }
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

        let mut idx = 0usize;
        let mut conformance = MessageConformance::default();
        let mut saw_cr = false;
        let mut line_start = 0;
        let mut max_line_len = 0;

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
                        if name_end.is_none() {
                            name_end.replace(idx);
                        }
                        state = State::Separator;
                    } else if c == b' ' || c == b'\t' {
                        if name_end.is_none() {
                            name_end.replace(idx);
                        }
                        conformance.set(MessageConformance::NAME_ENDS_WITH_SPACE, true);
                    } else if c == b'\n' {
                        // Got a newline before we finished parsing the name
                        conformance.set(MessageConformance::MISSING_COLON_VALUE, true);
                        name_end.replace(idx);
                        max_line_len = max_line_len.max(idx.saturating_sub(line_start));
                        value_start = idx;
                        value_end = idx;
                        idx += 1;
                        break;
                    } else if c != b'\r' && (c < 33 || c > 126) {
                        return Err(MailParsingError::HeaderParse(format!(
                            "header name must be comprised of printable US-ASCII characters. Found {c:?}"
                        )));
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
                            conformance.set(MessageConformance::NON_CANONICAL_LINE_ENDINGS, true);
                        }
                        state = State::NewLine;
                        saw_cr = false;
                        max_line_len = max_line_len.max(idx.saturating_sub(line_start));
                        line_start = idx + 1;
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

        max_line_len = max_line_len.max(idx.saturating_sub(line_start));
        if max_line_len > 78 {
            conformance.set(MessageConformance::LINE_TOO_LONG, true);
        }

        let name_end = name_end.unwrap_or_else(|| {
            conformance.set(MessageConformance::MISSING_COLON_VALUE, true);
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

    /// Re-constitute the header.
    /// The header value will be parsed out according to the known schema
    /// of the associated header name, and the parsed form used
    /// to build a new version of the header.
    /// This has the side effect of "fixing" non-conforming elements,
    /// but may come at the cost of "losing" the non-sensical or otherwise
    /// out of spec elements in the rebuilt header
    pub fn rebuild(&self) -> Result<Self> {
        let name = self.get_name();

        macro_rules! hdr {
            ($header_name:literal, $func_name:ident, encode) => {
                if name.eq_ignore_ascii_case($header_name) {
                    let value = self.$func_name().map_err(|err| {
                        MailParsingError::HeaderParse(format!(
                            "rebuilding '{name}' header: {err:#}"
                        ))
                    })?;
                    return Ok(Self::with_name_value($header_name, value.encode_value()));
                }
            };
            ($header_name:literal, unstructured) => {
                if name.eq_ignore_ascii_case($header_name) {
                    let value = self.as_unstructured().map_err(|err| {
                        MailParsingError::HeaderParse(format!(
                            "rebuilding '{name}' header: {err:#}"
                        ))
                    })?;
                    return Ok(Self::new_unstructured($header_name, value));
                }
            };
        }

        hdr!("From", as_mailbox_list, encode);
        hdr!("Resent-From", as_mailbox_list, encode);
        hdr!("Reply-To", as_address_list, encode);
        hdr!("To", as_address_list, encode);
        hdr!("Cc", as_address_list, encode);
        hdr!("Bcc", as_address_list, encode);
        hdr!("Resent-To", as_address_list, encode);
        hdr!("Resent-Cc", as_address_list, encode);
        hdr!("Resent-Bcc", as_address_list, encode);
        hdr!("Date", as_date, encode);
        hdr!("Sender", as_mailbox, encode);
        hdr!("Resent-Sender", as_mailbox, encode);
        hdr!("Message-ID", as_message_id, encode);
        hdr!("Content-ID", as_content_id, encode);
        hdr!("Content-Type", as_content_type, encode);
        hdr!(
            "Content-Transfer-Encoding",
            as_content_transfer_encoding,
            encode
        );
        hdr!("Content-Disposition", as_content_disposition, encode);
        hdr!("References", as_message_id_list, encode);
        hdr!("Subject", unstructured);
        hdr!("Comments", unstructured);
        hdr!("Mime-Version", unstructured);

        // Assume unstructured
        let value = self.as_unstructured().map_err(|err| {
            MailParsingError::HeaderParse(format!("rebuilding '{name}' header: {err:#}"))
        })?;
        Ok(Self::new_unstructured(name.to_string(), value))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::AddrSpec;

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
MessageConformance(
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
            conformance: MessageConformance(
                NON_CANONICAL_LINE_ENDINGS,
            ),
        },
        Header {
            name: "From",
            value: "Someone <someone@example.com>",
            separator: ":  ",
            conformance: MessageConformance(
                NON_CANONICAL_LINE_ENDINGS,
            ),
        },
    ],
}
"#
        );
    }

    #[test]
    fn as_mailbox() {
        let sender = Header::with_name_value("Sender", "John Smith <jsmith@example.com>");
        k9::snapshot!(
            sender.as_mailbox(),
            r#"
Ok(
    Mailbox {
        name: Some(
            "John Smith",
        ),
        address: AddrSpec {
            local_part: "jsmith",
            domain: "example.com",
        },
    },
)
"#
        );
    }

    #[test]
    fn assign_mailbox() {
        let mut sender = Header::with_name_value("Sender", "");
        sender.assign(Mailbox {
            name: Some("John Smith".to_string()),
            address: AddrSpec::new("john.smith", "example.com"),
        });
        assert_eq!(
            sender.to_header_string(),
            "Sender: John Smith <john.smith@example.com>\r\n"
        );

        sender.assign(Mailbox {
            name: Some("John \"the smith\" Smith".to_string()),
            address: AddrSpec::new("john.smith", "example.com"),
        });
        assert_eq!(
            sender.to_header_string(),
            "Sender: \"John \\\"the smith\\\" Smith\" <john.smith@example.com>\r\n"
        );
    }

    #[test]
    fn new_mailbox() {
        let sender = Header::new(
            "Sender",
            Mailbox {
                name: Some("John".to_string()),
                address: AddrSpec::new("john.smith", "example.com"),
            },
        );
        assert_eq!(
            sender.to_header_string(),
            "Sender: John <john.smith@example.com>\r\n"
        );

        let sender = Header::new(
            "Sender",
            Mailbox {
                name: Some("John".to_string()),
                address: AddrSpec::new("john smith", "example.com"),
            },
        );
        assert_eq!(
            sender.to_header_string(),
            "Sender: John <\"john smith\"@example.com>\r\n"
        );
    }

    #[test]
    fn new_mailbox_2047() {
        let sender = Header::new(
            "Sender",
            Mailbox {
                name: Some("André Pirard".to_string()),
                address: AddrSpec::new("andre", "example.com"),
            },
        );
        assert_eq!(
            sender.to_header_string(),
            "Sender: =?UTF-8?q?Andr=C3=A9_Pirard?= <andre@example.com>\r\n"
        );
    }

    #[test]
    fn test_unstructured_encode() {
        let header = Header::new_unstructured("Subject", "hello there");
        k9::snapshot!(header.value, "hello there");

        let header = Header::new_unstructured("Subject", "hello \"there\"");
        k9::snapshot!(header.value, "hello \"there\"");

        let header = Header::new_unstructured("Subject", "hello André Pirard");
        k9::snapshot!(header.value, "hello =?UTF-8?q?Andr=C3=A9?= Pirard");

        let header = Header::new_unstructured(
            "Subject",
            "hello there, this is a \
            longer header than the standard width and so it should \
            get wrapped in the produced value",
        );
        k9::snapshot!(
            header.to_header_string(),
            r#"
Subject: hello there, this is a longer header than the standard width and so it\r
\tshould get wrapped in the produced value\r

"#
        );

        let input_text = "hello there André, this is a longer header \
                          than the standard width and so it should get \
                          wrapped in the produced value. Do you hear me \
                          André? this should get really long!";
        let header = Header::new_unstructured("Subject", input_text);
        k9::snapshot!(
            header.to_header_string(),
            r#"
Subject: hello there =?UTF-8?q?Andr=C3=A9,?= this is a longer header than the standard width\r
\tand so it should get wrapped in the produced value. Do you hear me\r
\t=?UTF-8?q?Andr=C3=A9=3F?= this should get really long!\r

"#
        );

        k9::assert_equal!(header.as_unstructured().unwrap(), input_text);
    }

    #[test]
    fn test_date() {
        let header = Header::with_name_value("Date", "Tue, 1 Jul 2003 10:52:37 +0200");
        let date = header.as_date().unwrap();
        k9::snapshot!(date, "2003-07-01T10:52:37+02:00");
    }

    #[test]
    fn conformance_string() {
        k9::assert_equal!(
            MessageConformance::LINE_TOO_LONG.to_string(),
            "LINE_TOO_LONG"
        );
        k9::assert_equal!(
            (MessageConformance::LINE_TOO_LONG | MessageConformance::NEEDS_TRANSFER_ENCODING)
                .to_string(),
            "LINE_TOO_LONG|NEEDS_TRANSFER_ENCODING"
        );
        k9::assert_equal!(
            MessageConformance::from_str("").unwrap(),
            MessageConformance::default()
        );

        k9::assert_equal!(
            MessageConformance::from_str("LINE_TOO_LONG").unwrap(),
            MessageConformance::LINE_TOO_LONG
        );
        k9::assert_equal!(
            MessageConformance::from_str("LINE_TOO_LONG|MISSING_COLON_VALUE").unwrap(),
            MessageConformance::LINE_TOO_LONG | MessageConformance::MISSING_COLON_VALUE
        );
        k9::assert_equal!(
            MessageConformance::from_str("LINE_TOO_LONG|spoon").unwrap_err(),
            "invalid MessageConformance flag 'spoon', possible values are \
            'LINE_TOO_LONG', 'MISSING_COLON_VALUE', 'MISSING_DATE_HEADER', \
            'MISSING_MESSAGE_ID_HEADER', 'MISSING_MIME_VERSION', 'NAME_ENDS_WITH_SPACE', \
            'NEEDS_TRANSFER_ENCODING', 'NON_CANONICAL_LINE_ENDINGS'"
        );
    }
}
