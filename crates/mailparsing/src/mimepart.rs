use crate::header::{HeaderConformance, HeaderParseResult};
use crate::headermap::HeaderMap;
use crate::{Header, MailParsingError, MimeParameters, Result, SharedString};
use charset::Charset;
use std::convert::TryInto;
use std::str::FromStr;

/// Define our own because data_encoding::BASE64_MIME, despite its name,
/// is not RFC2045 compliant, and will not ignore spaces
const BASE64_RFC2045: data_encoding::Encoding = data_encoding_macro::new_encoding! {
    symbols: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    padding: '=',
    ignore: " \r\n\t",
    wrap_width: 76,
    wrap_separator: "\r\n",
};

#[derive(Debug, Clone)]
pub struct MimePart<'a> {
    /// The bytes that comprise this part, from its beginning to its end
    bytes: SharedString<'a>,
    /// The parsed headers from the start of bytes
    headers: HeaderMap<'a>,
    /// The index into bytes of the first non-header byte.
    body_offset: usize,
    body_len: usize,
    header_conformance: HeaderConformance,
    parts: Vec<Self>,
    /// For multipart, the content the precedes the first boundary
    intro: SharedString<'a>,
    /// For multipart, the content the follows the last boundary
    outro: SharedString<'a>,
}

struct Rfc2045Info {
    encoding: ContentTransferEncoding,
    charset: Charset,
    content_type: Option<MimeParameters>,
    is_text: bool,
    is_multipart: bool,
}

impl Rfc2045Info {
    fn new(headers: &HeaderMap, check_mime_version: bool) -> Result<Self> {
        if check_mime_version {
            if let Some(v) = headers.mime_version()? {
                if v != "1.0" {
                    return Err(MailParsingError::UnknownMimeVersion(v));
                }
            }
        }

        let content_transfer_encoding = headers.content_transfer_encoding()?;

        let encoding = match &content_transfer_encoding {
            Some(cte) => ContentTransferEncoding::from_str(&cte.value)?,
            None => ContentTransferEncoding::SevenBit,
        };

        let content_type = headers.content_type()?;
        let charset = if let Some(ct) = &content_type {
            ct.get("charset")
        } else {
            None
        };
        let charset = charset.unwrap_or_else(|| "us-ascii".to_string());

        let charset = Charset::for_label_no_replacement(charset.as_bytes())
            .ok_or_else(|| MailParsingError::BodyParse(format!("unsupported charset {charset}")))?;

        let (is_text, is_multipart) = if let Some(ct) = &content_type {
            (ct.is_text(), ct.is_multipart())
        } else {
            (true, false)
        };

        Ok(Self {
            encoding,
            charset,
            content_type,
            is_text,
            is_multipart,
        })
    }
}

impl<'a> MimePart<'a> {
    /// Parse some data into a tree of MimeParts
    pub fn parse<S: TryInto<SharedString<'a>>>(bytes: S) -> Result<Self> {
        let bytes = bytes.try_into().map_err(|_| MailParsingError::NotAscii)?;
        Self::parse_impl(bytes, true)
    }

    fn parse_impl(bytes: SharedString<'a>, check_mime_version: bool) -> Result<Self> {
        let HeaderParseResult {
            headers,
            body_offset,
            overall_conformance: header_conformance,
        } = Header::parse_headers(bytes.clone())?;

        let body_len = bytes.len();

        let mut part = Self {
            bytes,
            headers,
            body_offset,
            body_len,
            header_conformance,
            parts: vec![],
            intro: SharedString::Borrowed(""),
            outro: SharedString::Borrowed(""),
        };

        part.recursive_parse(check_mime_version)?;

        Ok(part)
    }

    fn recursive_parse(&mut self, check_mime_version: bool) -> Result<()> {
        let info = Rfc2045Info::new(&self.headers, check_mime_version)?;
        if let Some((boundary, true)) = info
            .content_type
            .as_ref()
            .and_then(|ct| ct.get("boundary").map(|b| (b, info.is_multipart)))
        {
            let boundary = format!("\n--{boundary}");
            let raw_body = self
                .bytes
                .slice(self.body_offset.saturating_sub(1)..self.bytes.len());

            let mut iter = memchr::memmem::find_iter(raw_body.as_bytes(), &boundary);
            if let Some(first_boundary_pos) = iter.next() {
                self.intro = raw_body.slice(0..first_boundary_pos);

                // When we create parts, we ignore the original body span in
                // favor of what we're parsing out here now
                self.body_len = 0;

                let mut boundary_end = first_boundary_pos + boundary.len();

                while let Some(part_start) =
                    memchr::memchr(b'\n', &raw_body.as_bytes()[boundary_end..])
                        .map(|p| p + boundary_end + 1)
                {
                    let part_end = iter
                        .next()
                        .map(|p| {
                            // P is the newline; we want to include it in the raw
                            // bytes for this part, so look beyond it
                            p + 1
                        })
                        .unwrap_or(raw_body.len());

                    let child = Self::parse_impl(raw_body.slice(part_start..part_end), false)?;
                    self.parts.push(child);

                    boundary_end = part_end -
                        1 /* newline we adjusted for when assigning part_end */
                        + boundary.len();

                    if boundary_end + 2 > raw_body.len() {
                        break;
                    }
                    if &raw_body.as_bytes()[boundary_end..boundary_end + 2] == b"--" {
                        if let Some(after_boundary) =
                            memchr::memchr(b'\n', &raw_body.as_bytes()[boundary_end..])
                                .map(|p| p + boundary_end + 1)
                        {
                            self.outro = raw_body.slice(after_boundary..raw_body.len());
                            eprintln!("outro is: '{}'", self.outro.escape_debug());
                        }
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    pub fn header_conformance(&self) -> HeaderConformance {
        self.header_conformance
    }

    /// Obtain a reference to the child parts
    pub fn child_parts(&self) -> &[Self] {
        &self.parts
    }

    /// Obtain a mutable reference to the child parts
    pub fn child_parts_mut(&mut self) -> &mut Vec<Self> {
        &mut self.parts
    }

    /// Obtains a reference to the headers
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Obtain a mutable reference to the headers
    pub fn headers_mut<'b>(&'b mut self) -> &'b mut HeaderMap<'a> {
        &mut self.headers
    }

    /// Get the raw, transfer-encoded body
    pub fn raw_body(&self) -> SharedString {
        self.bytes
            .slice(self.body_offset..self.body_len.max(self.body_offset))
    }

    /// Decode transfer decoding and return the body
    pub fn body(&self) -> Result<DecodedBody> {
        let info = Rfc2045Info::new(&self.headers, false)?;

        let bytes = match info.encoding {
            ContentTransferEncoding::Base64 => {
                let data = self.raw_body();
                let bytes = data.as_bytes();
                BASE64_RFC2045.decode(bytes).map_err(|err| {
                    let b = bytes[err.position] as char;
                    let region = &bytes[err.position.saturating_sub(8)..err.position + 8];
                    let region = String::from_utf8_lossy(region);
                    MailParsingError::BodyParse(format!(
                        "base64 decode: {err:#} b={b:?} in {region}"
                    ))
                })?
            }
            ContentTransferEncoding::QuotedPrintable => quoted_printable::decode(
                self.raw_body().as_bytes(),
                quoted_printable::ParseMode::Robust,
            )
            .map_err(|err| {
                MailParsingError::BodyParse(format!("quoted printable decode: {err:#}"))
            })?,
            ContentTransferEncoding::SevenBit
            | ContentTransferEncoding::EightBit
            | ContentTransferEncoding::Binary
                if info.is_text =>
            {
                return Ok(DecodedBody::Text(self.raw_body()));
            }
            ContentTransferEncoding::SevenBit | ContentTransferEncoding::EightBit => {
                let mut bytes = self.raw_body().as_bytes().to_vec();
                bytes.retain(|&b| !b.is_ascii_whitespace());
                return Ok(DecodedBody::Binary(bytes));
            }
            ContentTransferEncoding::Binary => {
                return Ok(DecodedBody::Binary(self.raw_body().as_bytes().to_vec()))
            }
        };

        let (decoded, _malformed) = info.charset.decode_without_bom_handling(&bytes);

        if info.is_text {
            Ok(DecodedBody::Text(decoded.to_string().into()))
        } else {
            Ok(DecodedBody::Binary(decoded.as_bytes().to_vec()))
        }
    }

    /// Write the message content to the provided output stream
    pub fn write_message<W: std::io::Write>(&self, out: &mut W) -> Result<()> {
        let line_ending = if self
            .header_conformance
            .contains(HeaderConformance::NON_CANONICAL_LINE_ENDINGS)
        {
            "\n"
        } else {
            "\r\n"
        };

        for hdr in self.headers.iter() {
            hdr.write_header(out)
                .map_err(|_| MailParsingError::WriteMessageIOError)?;
        }
        out.write_all(line_ending.as_bytes())
            .map_err(|_| MailParsingError::WriteMessageIOError)?;

        if self.parts.is_empty() {
            out.write_all(self.raw_body().as_bytes())
                .map_err(|_| MailParsingError::WriteMessageIOError)?;
        } else {
            let info = Rfc2045Info::new(&self.headers, false)?;
            let ct = info.content_type.ok_or_else(|| {
                MailParsingError::WriteMessageWtf(
                    "expected to have Content-Type when there are child parts",
                )
            })?;
            let boundary = ct.get("boundary").ok_or_else(|| {
                MailParsingError::WriteMessageWtf("expected Content-Type to have a boundary")
            })?;
            out.write_all(&self.intro.as_bytes())
                .map_err(|_| MailParsingError::WriteMessageIOError)?;
            for p in &self.parts {
                write!(out, "--{boundary}{line_ending}")
                    .map_err(|_| MailParsingError::WriteMessageIOError)?;
                p.write_message(out)?;
            }
            write!(out, "--{boundary}--{line_ending}")
                .map_err(|_| MailParsingError::WriteMessageIOError)?;
            out.write_all(&self.outro.as_bytes())
                .map_err(|_| MailParsingError::WriteMessageIOError)?;
        }
        Ok(())
    }

    /// Convenience method wrapping write_message that returns
    /// the formatted message as a standalone string
    pub fn to_message_string(&self) -> String {
        let mut out = vec![];
        self.write_message(&mut out).unwrap();
        String::from_utf8_lossy(&out).to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentTransferEncoding {
    SevenBit,
    EightBit,
    Binary,
    QuotedPrintable,
    Base64,
}

impl FromStr for ContentTransferEncoding {
    type Err = MailParsingError;

    fn from_str(s: &str) -> Result<Self> {
        if s.eq_ignore_ascii_case("7bit") {
            Ok(Self::SevenBit)
        } else if s.eq_ignore_ascii_case("8bit") {
            Ok(Self::EightBit)
        } else if s.eq_ignore_ascii_case("binary") {
            Ok(Self::Binary)
        } else if s.eq_ignore_ascii_case("quoted-printable") {
            Ok(Self::QuotedPrintable)
        } else if s.eq_ignore_ascii_case("base64") {
            Ok(Self::Base64)
        } else {
            Err(MailParsingError::InvalidContentTransferEncoding(
                s.to_string(),
            ))
        }
    }
}

#[derive(Debug)]
pub enum DecodedBody<'a> {
    Text(SharedString<'a>),
    Binary(Vec<u8>),
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
        k9::assert_equal!(message, part.to_message_string());
        assert_eq!(part.raw_body(), "I am the body");
        k9::snapshot!(
            part.body(),
            r#"
Ok(
    Text(
        "I am the body",
    ),
)
"#
        );
    }

    #[test]
    fn mime_encoded_body() {
        let message = concat!(
            "Subject: hello there\n",
            "From: Someone <someone@example.com>\n",
            "Mime-Version: 1.0\n",
            "Content-Type: text/plain\n",
            "Content-Transfer-Encoding: base64\n",
            "\n",
            "aGVsbG8K\n"
        );

        let part = MimePart::parse(message).unwrap();
        k9::assert_equal!(message, part.to_message_string());
        assert_eq!(part.raw_body(), "aGVsbG8K\n");
        k9::snapshot!(
            part.body(),
            r#"
Ok(
    Text(
        "hello
",
    ),
)
"#
        );
    }

    #[test]
    fn mime_multipart_1() {
        let message = concat!(
            "Subject: This is a test email\n",
            "Content-Type: multipart/alternative; boundary=foobar\n",
            "Mime-Version: 1.0\n",
            "Date: Sun, 02 Oct 2016 07:06:22 -0700 (PDT)\n",
            "\n",
            "--foobar\n",
            "Content-Type: text/plain; charset=utf-8\n",
            "Content-Transfer-Encoding: quoted-printable\n",
            "\n",
            "This is the plaintext version, in utf-8. Proof by Euro: =E2=82=AC\n",
            "--foobar\n",
            "Content-Type: text/html\n",
            "Content-Transfer-Encoding: base64\n",
            "\n",
            "PGh0bWw+PGJvZHk+VGhpcyBpcyB0aGUgPGI+SFRNTDwvYj4gdmVyc2lvbiwgaW4g \n",
            "dXMtYXNjaWkuIFByb29mIGJ5IEV1cm86ICZldXJvOzwvYm9keT48L2h0bWw+Cg== \n",
            "--foobar--\n",
            "After the final boundary stuff gets ignored.\n"
        );

        let part = MimePart::parse(message).unwrap();

        k9::assert_equal!(message, part.to_message_string());

        let children = part.child_parts();
        k9::assert_equal!(children.len(), 2);

        k9::snapshot!(
            children[0].body(),
            r#"
Ok(
    Text(
        "This is the plaintext version, in utf-8. Proof by Euro: €\r
",
    ),
)
"#
        );
        k9::snapshot!(
            children[1].body(),
            r#"
Ok(
    Text(
        "<html><body>This is the <b>HTML</b> version, in us-ascii. Proof by Euro: &euro;</body></html>
",
    ),
)
"#
        );
    }

    #[test]
    fn mutate_1() {
        let message = concat!(
            "Subject: This is a test email\r\n",
            "Content-Type: multipart/alternative; boundary=foobar\r\n",
            "Mime-Version: 1.0\r\n",
            "Date: Sun, 02 Oct 2016 07:06:22 -0700 (PDT)\r\n",
            "\r\n",
            "--foobar\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n",
            "\r\n",
            "This is the plaintext version, in utf-8. Proof by Euro: =E2=82=AC\r\n",
            "--foobar\r\n",
            "Content-Type: text/html\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "PGh0bWw+PGJvZHk+VGhpcyBpcyB0aGUgPGI+SFRNTDwvYj4gdmVyc2lvbiwgaW4g \r\n",
            "dXMtYXNjaWkuIFByb29mIGJ5IEV1cm86ICZldXJvOzwvYm9keT48L2h0bWw+Cg== \r\n",
            "--foobar--\r\n",
            "After the final boundary stuff gets ignored.\r\n"
        );

        let mut part = MimePart::parse(message).unwrap();
        k9::assert_equal!(message, part.to_message_string());
        fn munge(part: &mut MimePart) {
            let headers = part.headers_mut();
            headers.push(Header::with_name_value("X-Woot", "Hello"));
            headers.insert(0, Header::with_name_value("X-First", "at the top"));
            headers.retain(|hdr| !hdr.get_name().eq_ignore_ascii_case("date"));
        }
        munge(&mut part);

        let re_encoded = part.to_message_string();
        k9::snapshot!(
            re_encoded,
            r#"
X-First: at the top\r
Subject: This is a test email\r
Content-Type: multipart/alternative; boundary=foobar\r
Mime-Version: 1.0\r
X-Woot: Hello\r
\r
--foobar\r
Content-Type: text/plain; charset=utf-8\r
Content-Transfer-Encoding: quoted-printable\r
\r
This is the plaintext version, in utf-8. Proof by Euro: =E2=82=AC\r
--foobar\r
Content-Type: text/html\r
Content-Transfer-Encoding: base64\r
\r
PGh0bWw+PGJvZHk+VGhpcyBpcyB0aGUgPGI+SFRNTDwvYj4gdmVyc2lvbiwgaW4g \r
dXMtYXNjaWkuIFByb29mIGJ5IEV1cm86ICZldXJvOzwvYm9keT48L2h0bWw+Cg== \r
--foobar--\r
After the final boundary stuff gets ignored.\r

"#
        );

        eprintln!("part before mutate:\n{part:#?}");

        part.child_parts_mut().retain(|part| {
            let ct = part.headers().content_type().unwrap().unwrap();
            ct.value == "text/html"
        });

        eprintln!("part with html removed is:\n{part:#?}");

        let re_encoded = part.to_message_string();
        k9::snapshot!(
            re_encoded,
            r#"
X-First: at the top\r
Subject: This is a test email\r
Content-Type: multipart/alternative; boundary=foobar\r
Mime-Version: 1.0\r
X-Woot: Hello\r
\r
--foobar\r
Content-Type: text/html\r
Content-Transfer-Encoding: base64\r
\r
PGh0bWw+PGJvZHk+VGhpcyBpcyB0aGUgPGI+SFRNTDwvYj4gdmVyc2lvbiwgaW4g \r
dXMtYXNjaWkuIFByb29mIGJ5IEV1cm86ICZldXJvOzwvYm9keT48L2h0bWw+Cg== \r
--foobar--\r
After the final boundary stuff gets ignored.\r

"#
        );
    }
}
