use crate::header::{HeaderConformance, HeaderParseResult};
use crate::headermap::HeaderMap;
use crate::{Header, MailParsingError, MimeParameters, Result, SharedString};
use charset::Charset;
use std::str::FromStr;

pub struct MimePart<'a> {
    /// The bytes that comprise this part, from its beginning to its end
    bytes: SharedString<'a>,
    /// The parsed headers from the start of bytes
    headers: HeaderMap<'a>,
    /// The index into bytes of the first non-header byte.
    body_offset: usize,
    overall_conformance: HeaderConformance,
}

struct Rfc2045Info {
    encoding: ContentTransferEncoding,
    charset: Charset,
    content_type: Option<MimeParameters>,
    content_transfer_encoding: Option<MimeParameters>,
    is_text: bool,
    is_multipart: bool,
}

impl Rfc2045Info {
    fn new(headers: &HeaderMap) -> Result<Self> {
        if let Some(v) = headers.mime_version()? {
            if v != "1.0" {
                return Err(MailParsingError::UnknownMimeVersion(v));
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

            let charset =
                Charset::for_label_no_replacement(charset.as_bytes()).ok_or_else(|| {
                    MailParsingError::BodyParse(format!("unsupported charset {charset}"))
                })?;

            let (is_text, is_multipart) = if let Some(ct) = &content_type {
                (ct.is_text(), ct.is_multipart())
            } else {
                (true, false)
            };

            Ok(Self {
                encoding,
                charset,
                content_type,
                content_transfer_encoding,
                is_text,
                is_multipart,
            })
        } else {
            // Just assume text/plain, us-ascii
            Ok(Self {
                encoding: ContentTransferEncoding::SevenBit,
                charset: Charset::for_label_no_replacement(b"us-ascii")
                    .expect("ASCII to be available"),
                content_type: None,
                content_transfer_encoding: None,
                is_text: true,
                is_multipart: false,
            })
        }
    }
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

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn raw_body(&self) -> SharedString {
        self.bytes.slice(self.body_offset..self.bytes.len())
    }

    pub fn body(&self) -> Result<DecodedBody> {
        let info = Rfc2045Info::new(&self.headers)?;

        let bytes = match info.encoding {
            ContentTransferEncoding::Base64 => data_encoding::BASE64_MIME
                .decode(self.raw_body().as_bytes())
                .map_err(|err| MailParsingError::BodyParse(format!("base64 decode: {err:#}")))?,
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
                bytes.retain(|&b| b != b'\r' && b != b'\n');
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
}
