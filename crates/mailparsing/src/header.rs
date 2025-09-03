use crate::headermap::{EncodeHeaderValue, HeaderMap};
use crate::rfc5322_parser::Parser;
use crate::strings::IntoSharedString;
use crate::{
    AddressList, AuthenticationResults, MailParsingError, Mailbox, MailboxList, MessageID,
    MimeParameters, Result, SharedString,
};
use chrono::{DateTime, FixedOffset};
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

    pub fn to_owned(&'a self) -> Header<'static> {
        Header {
            name: self.name.to_owned(),
            value: self.value.to_owned(),
            separator: self.separator.to_owned(),
            conformance: self.conformance,
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

        let value = if value.is_ascii() {
            crate::textwrap::wrap(&value)
        } else {
            crate::rfc5322_parser::qp_encode(&value)
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

    pub fn parse_headers<S>(header_block: S) -> Result<HeaderParseResult<'a>>
    where
        S: IntoSharedString<'a>,
    {
        let (header_block, mut overall_conformance) = header_block.into_shared_string();
        let mut headers = vec![];
        let mut idx = 0;

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
            if headers.is_empty() && b.is_ascii_whitespace() {
                return Err(MailParsingError::HeaderParse(
                    "header block must not start with spaces".to_string(),
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

        let mut idx = 0usize;
        let mut conformance = MessageConformance::default();
        let mut saw_cr = false;
        let mut line_start = 0;
        let mut max_line_len = 0;

        loop {
            match state {
                State::Initial => {
                    if c.is_ascii_whitespace() {
                        return Err(MailParsingError::HeaderParse(
                            "header cannot start with space".to_string(),
                        ));
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
                    } else if c != b'\r' && !(33..=126).contains(&c) {
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
        let value = header_block.slice(value_start..value_end.max(value_start));
        let separator = header_block.slice(name_end..value_start.max(name_end));

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
    fn test_spacing_roundtrip() {
        let (header, _size) = Header::parse(
            "Subject: =?UTF-8?q?=D8=AA=D8=B3=D8=AA_=DB=8C=DA=A9_=D8=AF=D9=88_=D8=B3=D9=87?=",
        )
        .unwrap();
        k9::snapshot!(
            header.as_unstructured(),
            r#"
Ok(
    "تست یک دو سه",
)
"#
        );

        let rebuilt = header.rebuild().unwrap();
        k9::snapshot!(
            rebuilt.as_unstructured(),
            r#"
Ok(
    "تست یک دو سه",
)
"#
        );
    }

    #[test]
    fn test_unstructured_encode() {
        let header = Header::new_unstructured("Subject", "hello there");
        k9::snapshot!(header.value, "hello there");

        let header = Header::new_unstructured("Subject", "hello \"there\"");
        k9::snapshot!(header.value, "hello \"there\"");

        let header = Header::new_unstructured("Subject", "hello André Pirard");
        k9::snapshot!(header.value, "=?UTF-8?q?hello_Andr=C3=A9_Pirard?=");

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
Subject: =?UTF-8?q?hello_there_Andr=C3=A9,_this_is_a_longer_header_than_the_sta?=\r
\t=?UTF-8?q?ndard_width_and_so_it_should_get_wrapped_in_the_produced_val?=\r
\t=?UTF-8?q?ue._Do_you_hear_me_Andr=C3=A9=3F_this_should_get_really_long?=\r
\t=?UTF-8?q?!?=\r

"#
        );

        k9::assert_equal!(header.as_unstructured().unwrap(), input_text);
    }

    #[test]
    fn test_unstructured_encode_farsi() {
        let farsi_input = "بوت‌كمپ قدرت نوشتن رهنماکالج";
        let header = Header::new_unstructured("Subject", farsi_input);
        eprintln!("{}", header.value);
        k9::assert_equal!(header.as_unstructured().unwrap(), farsi_input);
    }

    #[test]
    fn test_wrapping_in_from_header() {
        let header = Header::new_unstructured(
            "From",
            "=?UTF-8?q?=D8=B1=D9=87=D9=86=D9=85=D8=A7_=DA=A9=D8=A7=D9=84=D8=AC?= \
            <from-dash-wrap-me@example.com>",
        );

        eprintln!("made: {}", header.to_header_string());

        // In the original problem report, the header got wrapped onto a second
        // line at `from-` which broke parsing the header as a mailbox.
        // This line asserts that it does parse.
        let _ = header.as_mailbox_list().unwrap();
    }

    #[test]
    fn test_multi_line_filename() {
        let header = Header::with_name_value(
            "Content-Disposition",
            "attachment;\r\n\
            \tfilename*0*=UTF-8''%D0%A7%D0%B0%D1%81%D1%82%D0%B8%D0%BD%D0%B0%20%D0%B2;\r\n\
            \tfilename*1*=%D0%BA%D0%BB%D0%B0%D0%B4%D0%B5%D0%BD%D0%BE%D0%B3%D0%BE%20;\r\n\
            \tfilename*2*=%D0%BF%D0%BE%D0%B2%D1%96%D0%B4%D0%BE%D0%BC%D0%BB%D0%B5%D0%BD;\r\n\
            \tfilename*3*=%D0%BD%D1%8F",
        );

        match header.as_content_disposition() {
            Ok(cd) => {
                k9::snapshot!(
                    cd.get("filename"),
                    r#"
Some(
    "Частина вкладеного повідомлення",
)
"#
                );
            }
            Err(err) => {
                eprintln!("{err:#}");
                panic!("expected to parse");
            }
        }
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

    #[test]
    fn split_qp_display_name() {
        let header = Header::with_name_value("From", "=?UTF-8?q?=D8=A7=D9=86=D8=AA=D8=B4=D8=A7=D8=B1=D8=A7=D8=AA_=D8=AC=D9=85?=\r\n\t=?UTF-8?q?=D8=A7=D9=84?= <noreply@email.ahasend.com>");

        let mbox = header.as_mailbox_list().unwrap();

        k9::snapshot!(
            mbox,
            r#"
MailboxList(
    [
        Mailbox {
            name: Some(
                "انتشارات جم ال",
            ),
            address: AddrSpec {
                local_part: "noreply",
                domain: "email.ahasend.com",
            },
        },
    ],
)
"#
        );
    }
}
