use crate::header::{HeaderConformance, HeaderParseResult};
use crate::headermap::HeaderMap;
use crate::{Header, MailParsingError, MessageID, MimeParameters, Result, SharedString};
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

#[derive(Debug, Clone, PartialEq)]
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
    attachment_options: Option<AttachmentOptions>,
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

        let content_disposition = headers.content_disposition()?;
        let attachment_options = match content_disposition {
            Some(cd) => {
                let inline = cd.value == "inline";
                let content_id = headers.content_id()?;
                let file_name = cd.get("filename");

                Some(AttachmentOptions {
                    file_name,
                    inline,
                    content_id: content_id.map(|cid| cid.0),
                })
            }
            None => None,
        };

        Ok(Self {
            encoding,
            charset,
            content_type,
            is_text,
            is_multipart,
            attachment_options,
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

    /// Re-constitute the message.
    /// Each element will be parsed out, and the parsed form used
    /// to build a new message.
    /// This has the side effect of "fixing" non-conforming elements,
    /// but may come at the cost of "losing" the non-sensical or otherwise
    /// out of spec elements in the rebuilt message
    pub fn rebuild(&self) -> Result<Self> {
        let info = Rfc2045Info::new(&self.headers, false)?;

        let mut children = vec![];
        for part in &self.parts {
            children.push(part.rebuild()?);
        }

        let mut rebuilt = if children.is_empty() {
            let body = self.body()?;
            match body {
                DecodedBody::Text(text) => {
                    let ct = info
                        .content_type
                        .as_ref()
                        .map(|ct| ct.value.as_str())
                        .unwrap_or("text/plain");
                    Self::new_text(ct, text.as_str())
                }
                DecodedBody::Binary(data) => {
                    let ct = info
                        .content_type
                        .as_ref()
                        .map(|ct| ct.value.as_str())
                        .unwrap_or("application/octet-stream");
                    Self::new_binary(ct, &data, info.attachment_options.as_ref())
                }
            }
        } else {
            let ct = info.content_type.ok_or_else(|| {
                MailParsingError::BodyParse(format!(
                    "multipart message has no content-type information!?"
                ))
            })?;
            Self::new_multipart(&ct.value, children, ct.get("boundary").as_deref())
        };

        for hdr in self.headers.iter() {
            // Skip rfc2045 associated headers; we already rebuilt
            // those above
            let name = hdr.get_name();
            if name.eq_ignore_ascii_case("Content-Type")
                || name.eq_ignore_ascii_case("Content-Transfer-Encoding")
                || name.eq_ignore_ascii_case("Content-Disposition")
                || name.eq_ignore_ascii_case("Content-ID")
            {
                continue;
            }

            rebuilt.headers_mut().push(hdr.rebuild()?);
        }

        Ok(rebuilt)
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

    /// Constructs a new part with textual utf8 content.
    /// quoted-printable transfer encoding will be applied,
    /// unless it is smaller to represent the text in base64
    pub fn new_text(content_type: &str, content: &str) -> Self {
        // We'll probably use qp, so speculatively do the work
        let qp_encoded = quoted_printable::encode(content);

        let (mut encoded, encoding) =
            if qp_encoded.len() <= BASE64_RFC2045.encode_len(content.len()) {
                (qp_encoded, "quoted-printable")
            } else {
                // Turns out base64 will be smaller; perhaps the content
                // is dominated by non-ASCII text?
                (
                    BASE64_RFC2045.encode(content.as_bytes()).into_bytes(),
                    "base64",
                )
            };
        if !encoded.ends_with(b"\r\n") {
            encoded.extend_from_slice(b"\r\n");
        }
        let mut headers = HeaderMap::default();

        let mut ct = MimeParameters::new(content_type);
        ct.set(
            "charset",
            if content.is_ascii() {
                "us-ascii"
            } else {
                "utf-8"
            },
        );
        headers.set_content_type(ct);

        headers.set_content_transfer_encoding(MimeParameters::new(encoding));

        let body_len = encoded.len();
        let bytes =
            String::from_utf8(encoded).expect("transfer encoder to produce valid ASCII output");

        Self {
            bytes: bytes.into(),
            headers,
            body_offset: 0,
            body_len,
            header_conformance: HeaderConformance::default(),
            parts: vec![],
            intro: "".into(),
            outro: "".into(),
        }
    }

    pub fn new_text_plain(content: &str) -> Self {
        Self::new_text("text/plain", content)
    }

    pub fn new_html(content: &str) -> Self {
        Self::new_text("text/html", content)
    }

    pub fn new_multipart(content_type: &str, parts: Vec<Self>, boundary: Option<&str>) -> Self {
        let mut headers = HeaderMap::default();

        let mut ct = MimeParameters::new(content_type);
        match boundary {
            Some(b) => {
                ct.set("boundary", b);
            }
            None => {
                // Generate a random boundary
                let uuid = uuid::Uuid::new_v4();
                let boundary = data_encoding::BASE64_NOPAD.encode(uuid.as_bytes());
                ct.set("boundary", &boundary);
            }
        }
        headers.set_content_type(ct);

        Self {
            bytes: "".into(),
            headers,
            body_offset: 0,
            body_len: 0,
            header_conformance: HeaderConformance::default(),
            parts,
            intro: "".into(),
            outro: "".into(),
        }
    }

    pub fn new_binary(
        content_type: &str,
        content: &[u8],
        options: Option<&AttachmentOptions>,
    ) -> Self {
        let mut encoded = BASE64_RFC2045.encode(content);
        if !encoded.ends_with("\r\n") {
            encoded.push_str("\r\n");
        }
        let mut headers = HeaderMap::default();

        headers.set_content_type(MimeParameters::new(content_type));
        headers.set_content_transfer_encoding(MimeParameters::new("base64"));

        if let Some(opts) = options {
            let mut cd = MimeParameters::new(if opts.inline { "inline" } else { "attachment" });
            if let Some(name) = &opts.file_name {
                cd.set("filename", name);
            }
            headers.set_content_disposition(cd);

            if let Some(id) = &opts.content_id {
                headers.set_content_id(MessageID(id.to_string()));
            }
        }

        let body_len = encoded.len();

        Self {
            bytes: encoded.into(),
            headers,
            body_offset: 0,
            body_len,
            header_conformance: HeaderConformance::default(),
            parts: vec![],
            intro: "".into(),
            outro: "".into(),
        }
    }

    /// Returns a SimplifiedStructure representation of the mime tree,
    /// with the (probable) primary text/plain and text/html parts
    /// pulled out, and the remaining parts recorded as a flat
    /// attachments array
    pub fn simplified_structure(&'a self) -> Result<SimplifiedStructure<'a>> {
        let info = Rfc2045Info::new(&self.headers, false)?;

        if let Some(ct) = &info.content_type {
            if ct.value == "text/plain" {
                return Ok(SimplifiedStructure {
                    text: match self.body()? {
                        DecodedBody::Text(t) => Some(t),
                        DecodedBody::Binary(_) => {
                            return Err(MailParsingError::BodyParse(
                                "expected text/plain part to be text, but it is binary".to_string(),
                            ))
                        }
                    },
                    html: None,
                    headers: &self.headers,
                    attachments: vec![],
                });
            }
            if ct.value == "text/html" {
                return Ok(SimplifiedStructure {
                    html: match self.body()? {
                        DecodedBody::Text(t) => Some(t),
                        DecodedBody::Binary(_) => {
                            return Err(MailParsingError::BodyParse(
                                "expected text/html part to be text, but it is binary".to_string(),
                            ))
                        }
                    },
                    text: None,
                    headers: &self.headers,
                    attachments: vec![],
                });
            }
            if ct.value.starts_with("multipart/") {
                let mut text = None;
                let mut html = None;
                let mut attachments = vec![];

                for p in &self.parts {
                    if let Ok(mut s) = p.simplified_structure() {
                        if s.text.is_some() && text.is_none() {
                            text = s.text;
                        }
                        if s.html.is_some() && html.is_none() {
                            html = s.html;
                        }
                        attachments.append(&mut s.attachments);
                    }
                }

                return Ok(SimplifiedStructure {
                    html,
                    text,
                    headers: &self.headers,
                    attachments,
                });
            }

            return Ok(SimplifiedStructure {
                html: None,
                text: None,
                headers: &self.headers,
                attachments: vec![self.clone()],
            });
        }

        // Assume text/plain content-type
        Ok(SimplifiedStructure {
            text: match self.body()? {
                DecodedBody::Text(t) => Some(t),
                DecodedBody::Binary(_) => {
                    return Err(MailParsingError::BodyParse(
                        "expected text/plain part to be text, but it is binary".to_string(),
                    ))
                }
            },
            html: None,
            headers: &self.headers,
            attachments: vec![],
        })
    }
}

#[derive(Debug, Clone)]
pub struct SimplifiedStructure<'a> {
    pub text: Option<SharedString<'a>>,
    pub html: Option<SharedString<'a>>,
    pub headers: &'a HeaderMap<'a>,
    pub attachments: Vec<MimePart<'a>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentOptions {
    pub file_name: Option<String>,
    pub inline: bool,
    pub content_id: Option<String>,
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

#[derive(Debug, PartialEq)]
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

        k9::snapshot!(
            part.rebuild().unwrap().to_message_string(),
            r#"
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Content-Transfer-Encoding: quoted-printable\r
Subject: hello there\r
From: Someone <someone@example.com>\r
\r
I am the body\r

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

        k9::snapshot!(
            part.rebuild().unwrap().to_message_string(),
            r#"
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Content-Transfer-Encoding: quoted-printable\r
Subject: hello there\r
From: Someone <someone@example.com>\r
Mime-Version: 1.0\r
\r
hello=0A\r

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

    #[test]
    fn construct_1() {
        let input_text = "Well, hello there! This is the plaintext version, in utf-8. Here's a Euro: €, and here are some emoji 👻 🍉 💩 and this long should be long enough that we wrap it in the returned part, let's see how that turns out!\r\n";

        let part = MimePart::new_text_plain(input_text);

        let encoded = part.to_message_string();
        k9::snapshot!(
            &encoded,
            r#"
Content-Type: text/plain;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
\r
Well, hello there! This is the plaintext version, in utf-8. Here's a Euro: =\r
=E2=82=AC, and here are some emoji =F0=9F=91=BB =F0=9F=8D=89 =F0=9F=92=A9 a=\r
nd this long should be long enough that we wrap it in the returned part, le=\r
t's see how that turns out!\r

"#
        );

        let parsed_part = MimePart::parse(encoded.clone()).unwrap();
        k9::assert_equal!(encoded.as_str(), parsed_part.to_message_string().as_str());
        k9::assert_equal!(part.body().unwrap(), DecodedBody::Text(input_text.into()));
    }

    #[test]
    fn construct_2() {
        let msg = MimePart::new_multipart(
            "multipart/mixed",
            vec![
                MimePart::new_text_plain("plain text"),
                MimePart::new_html("<b>rich</b> text"),
                MimePart::new_binary(
                    "application/octet-stream",
                    &[0, 1, 2, 3],
                    Some(&AttachmentOptions {
                        file_name: Some("woot.bin".to_string()),
                        inline: false,
                        content_id: Some("woot.id".to_string()),
                    }),
                ),
            ],
            Some("my-boundary"),
        );
        k9::snapshot!(
            msg.to_message_string(),
            r#"
Content-Type: multipart/mixed;\r
\tboundary="my-boundary"\r
\r
--my-boundary\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Content-Transfer-Encoding: quoted-printable\r
\r
plain text\r
--my-boundary\r
Content-Type: text/html;\r
\tcharset="us-ascii"\r
Content-Transfer-Encoding: quoted-printable\r
\r
<b>rich</b> text\r
--my-boundary\r
Content-Type: application/octet-stream\r
Content-Transfer-Encoding: base64\r
Content-Disposition: attachment;\r
\tfilename="woot.bin"\r
Content-ID: <woot.id>\r
\r
AAECAw==\r
--my-boundary--\r

"#
        );
    }
}
