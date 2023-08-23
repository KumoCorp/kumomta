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
                self.body_len = first_boundary_pos;

                let mut boundary_end = first_boundary_pos + boundary.len();

                while let Some(part_start) =
                    memchr::memchr(b'\n', &raw_body.as_bytes()[boundary_end..])
                        .map(|p| p + boundary_end + 1)
                {
                    let part_end = iter.next().unwrap_or(raw_body.len());

                    let child = Self::parse_impl(raw_body.slice(part_start..part_end), false)?;
                    self.parts.push(child);

                    boundary_end = part_end + boundary.len();
                    if boundary_end + 2 > raw_body.len()
                        || &raw_body.as_bytes()[boundary_end..boundary_end + 2] == b"--"
                    {
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
    pub fn headers_mut(&'a mut self) -> &'a mut HeaderMap {
        &mut self.headers
    }

    /// Get the raw, transfer-encoded body
    pub fn raw_body(&self) -> SharedString {
        self.bytes.slice(self.body_offset..self.body_len)
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
        let children = part.child_parts();
        k9::assert_equal!(children.len(), 2);

        k9::snapshot!(
            children[0].body(),
            r#"
Ok(
    Text(
        "This is the plaintext version, in utf-8. Proof by Euro: €",
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
}
