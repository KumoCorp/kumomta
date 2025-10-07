use crate::header::{HeaderParseResult, MessageConformance};
use crate::headermap::HeaderMap;
use crate::strings::IntoSharedString;
use crate::{
    has_lone_cr_or_lf, Header, MailParsingError, MessageID, MimeParameterEncoding, MimeParameters,
    Result, SharedString,
};
use charset::Charset;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
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
    conformance: MessageConformance,
    parts: Vec<Self>,
    /// For multipart, the content the precedes the first boundary
    intro: SharedString<'a>,
    /// For multipart, the content the follows the last boundary
    outro: SharedString<'a>,
}

#[derive(PartialEq, Debug)]
pub struct Rfc2045Info {
    pub encoding: ContentTransferEncoding,
    pub charset: Result<Charset>,
    pub content_type: Option<MimeParameters>,
    pub is_text: bool,
    pub is_multipart: bool,
    pub attachment_options: Option<AttachmentOptions>,
    pub invalid_mime_headers: bool,
}

impl Rfc2045Info {
    // This must be infallible so that a basic mime structure can be parsed
    // even if the mime headers are a bit borked
    fn new(headers: &HeaderMap) -> Self {
        let mut invalid_mime_headers = false;
        let encoding = match headers.content_transfer_encoding() {
            Ok(Some(cte)) => match ContentTransferEncoding::from_str(&cte.value) {
                Ok(encoding) => encoding,
                Err(_) => {
                    invalid_mime_headers = true;
                    ContentTransferEncoding::SevenBit
                }
            },
            Ok(None) => ContentTransferEncoding::SevenBit,
            Err(_) => {
                invalid_mime_headers = true;
                ContentTransferEncoding::SevenBit
            }
        };

        let content_type = match headers.content_type() {
            Ok(ct) => ct,
            Err(_) => {
                invalid_mime_headers = true;
                None
            }
        };

        let mut ct_name = None;
        let charset = if let Some(ct) = &content_type {
            ct_name = ct.get("name");
            ct.get("charset")
        } else {
            None
        };
        let charset = charset.unwrap_or_else(|| "us-ascii".to_string());

        let charset = Charset::for_label_no_replacement(charset.as_bytes())
            .ok_or_else(|| MailParsingError::BodyParse(format!("unsupported charset {charset}")));

        let (is_text, is_multipart) = if let Some(ct) = &content_type {
            (ct.is_text(), ct.is_multipart())
        } else {
            (true, false)
        };

        let mut inline = false;
        let mut cd_file_name = None;

        match headers.content_disposition() {
            Ok(Some(cd)) => {
                inline = cd.value == "inline";
                cd_file_name = cd.get("filename");
            }
            Ok(None) => {}
            Err(_) => {
                invalid_mime_headers = true;
            }
        };

        let content_id = match headers.content_id() {
            Ok(cid) => cid.map(|cid| cid.0),
            Err(_) => {
                invalid_mime_headers = true;
                None
            }
        };

        let file_name = match (cd_file_name, ct_name) {
            (Some(name), _) | (None, Some(name)) => Some(name),
            (None, None) => None,
        };

        let attachment_options = if inline || file_name.is_some() || content_id.is_some() {
            Some(AttachmentOptions {
                file_name,
                inline,
                content_id,
            })
        } else {
            None
        };

        Self {
            encoding,
            charset,
            content_type,
            is_text,
            is_multipart,
            attachment_options,
            invalid_mime_headers,
        }
    }

    pub fn content_type(&self) -> Option<&str> {
        self.content_type
            .as_ref()
            .map(|params| params.value.as_str())
    }
}

impl<'a> MimePart<'a> {
    /// Parse some data into a tree of MimeParts
    pub fn parse<S>(bytes: S) -> Result<Self>
    where
        S: IntoSharedString<'a>,
    {
        let (bytes, base_conformance) = bytes.into_shared_string();
        Self::parse_impl(bytes, base_conformance, true)
    }

    /// Obtain a version of self that has a static lifetime
    pub fn to_owned(&self) -> MimePart<'static> {
        MimePart {
            bytes: self.bytes.to_owned(),
            headers: self.headers.to_owned(),
            body_offset: self.body_offset,
            body_len: self.body_len,
            conformance: self.conformance,
            parts: self.parts.iter().map(|p| p.to_owned()).collect(),
            intro: self.intro.to_owned(),
            outro: self.outro.to_owned(),
        }
    }

    fn parse_impl(
        bytes: SharedString<'a>,
        base_conformance: MessageConformance,
        is_top_level: bool,
    ) -> Result<Self> {
        let HeaderParseResult {
            headers,
            body_offset,
            overall_conformance: mut conformance,
        } = Header::parse_headers(bytes.clone())?;

        conformance |= base_conformance;

        let body_len = bytes.len();

        if !bytes.is_ascii() {
            conformance.set(MessageConformance::NEEDS_TRANSFER_ENCODING, true);
        }
        {
            let mut prev = 0;
            for idx in memchr::memchr_iter(b'\n', bytes.as_bytes()) {
                if idx - prev > 78 {
                    conformance.set(MessageConformance::LINE_TOO_LONG, true);
                    break;
                }
                prev = idx;
            }
        }
        conformance.set(
            MessageConformance::NON_CANONICAL_LINE_ENDINGS,
            has_lone_cr_or_lf(bytes.as_bytes()),
        );

        if is_top_level {
            conformance.set(
                MessageConformance::MISSING_DATE_HEADER,
                !matches!(headers.date(), Ok(Some(_))),
            );
            conformance.set(
                MessageConformance::MISSING_MESSAGE_ID_HEADER,
                !matches!(headers.message_id(), Ok(Some(_))),
            );
            conformance.set(
                MessageConformance::MISSING_MIME_VERSION,
                match headers.mime_version() {
                    Ok(Some(v)) => v.as_str() != "1.0",
                    _ => true,
                },
            );
        }

        let mut part = Self {
            bytes,
            headers,
            body_offset,
            body_len,
            conformance,
            parts: vec![],
            intro: SharedString::Borrowed(""),
            outro: SharedString::Borrowed(""),
        };

        part.recursive_parse()?;

        Ok(part)
    }

    fn recursive_parse(&mut self) -> Result<()> {
        let info = Rfc2045Info::new(&self.headers);
        if info.invalid_mime_headers {
            self.conformance |= MessageConformance::INVALID_MIME_HEADERS;
        }
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

                    let child = Self::parse_impl(
                        raw_body.slice(part_start..part_end),
                        MessageConformance::default(),
                        false,
                    )?;
                    self.conformance |= child.conformance;
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

    pub fn conformance(&self) -> MessageConformance {
        self.conformance
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
    pub fn headers(&'_ self) -> &'_ HeaderMap<'_> {
        &self.headers
    }

    /// Obtain a mutable reference to the headers
    pub fn headers_mut<'b>(&'b mut self) -> &'b mut HeaderMap<'a> {
        &mut self.headers
    }

    /// Get the raw, transfer-encoded body
    pub fn raw_body(&'_ self) -> SharedString<'_> {
        self.bytes
            .slice(self.body_offset..self.body_len.max(self.body_offset))
    }

    pub fn rfc2045_info(&self) -> Rfc2045Info {
        Rfc2045Info::new(&self.headers)
    }

    /// Decode transfer decoding and return the body
    pub fn body(&'_ self) -> Result<DecodedBody<'_>> {
        let info = Rfc2045Info::new(&self.headers);

        let bytes = match info.encoding {
            ContentTransferEncoding::Base64 => {
                let data = self.raw_body();
                let bytes = data.as_bytes();
                BASE64_RFC2045.decode(bytes).map_err(|err| {
                    let b = bytes[err.position] as char;
                    let region =
                        &bytes[err.position.saturating_sub(8)..(err.position + 8).min(bytes.len())];
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
            ContentTransferEncoding::SevenBit
            | ContentTransferEncoding::EightBit
            | ContentTransferEncoding::Binary => {
                return Ok(DecodedBody::Binary(self.raw_body().as_bytes().to_vec()))
            }
        };

        if info.is_text {
            let (decoded, _malformed) = info.charset?.decode_without_bom_handling(&bytes);
            Ok(DecodedBody::Text(decoded.to_string().into()))
        } else {
            Ok(DecodedBody::Binary(bytes))
        }
    }

    /// Re-constitute the message.
    /// Each element will be parsed out, and the parsed form used
    /// to build a new message.
    /// This has the side effect of "fixing" non-conforming elements,
    /// but may come at the cost of "losing" the non-sensical or otherwise
    /// out of spec elements in the rebuilt message
    pub fn rebuild(&self) -> Result<Self> {
        let info = Rfc2045Info::new(&self.headers);

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
                    Self::new_text(ct, text.as_str())?
                }
                DecodedBody::Binary(data) => {
                    let ct = info
                        .content_type
                        .as_ref()
                        .map(|ct| ct.value.as_str())
                        .unwrap_or("application/octet-stream");
                    Self::new_binary(ct, &data, info.attachment_options.as_ref())?
                }
            }
        } else {
            let ct = info.content_type.ok_or_else(|| {
                MailParsingError::BodyParse(
                    "multipart message has no content-type information!?".to_string(),
                )
            })?;
            Self::new_multipart(&ct.value, children, ct.get("boundary").as_deref())?
        };

        for hdr in self.headers.iter() {
            let name = hdr.get_name();
            if name.eq_ignore_ascii_case("Content-ID") {
                continue;
            }

            // Merge in any MimeParameters that we might otherwise have lost
            // in the rebuild
            if name.eq_ignore_ascii_case("Content-Type") {
                if let Ok(params) = hdr.as_content_type() {
                    let Some(mut dest) = rebuilt.headers_mut().content_type()? else {
                        continue;
                    };

                    for (k, v) in params.parameter_map() {
                        if dest.get(&k).is_none() {
                            dest.set(&k, &v);
                        }
                    }

                    rebuilt.headers_mut().set_content_type(dest)?;
                }
                continue;
            }
            if name.eq_ignore_ascii_case("Content-Transfer-Encoding") {
                if let Ok(params) = hdr.as_content_transfer_encoding() {
                    let Some(mut dest) = rebuilt.headers_mut().content_transfer_encoding()? else {
                        continue;
                    };

                    for (k, v) in params.parameter_map() {
                        if dest.get(&k).is_none() {
                            dest.set(&k, &v);
                        }
                    }

                    rebuilt.headers_mut().set_content_transfer_encoding(dest)?;
                }
                continue;
            }
            if name.eq_ignore_ascii_case("Content-Disposition") {
                if let Ok(params) = hdr.as_content_disposition() {
                    let Some(mut dest) = rebuilt.headers_mut().content_disposition()? else {
                        continue;
                    };

                    for (k, v) in params.parameter_map() {
                        if dest.get(&k).is_none() {
                            dest.set(&k, &v);
                        }
                    }

                    rebuilt.headers_mut().set_content_disposition(dest)?;
                }
                continue;
            }

            if let Ok(hdr) = hdr.rebuild() {
                rebuilt.headers_mut().push(hdr);
            }
        }

        Ok(rebuilt)
    }

    /// Write the message content to the provided output stream
    pub fn write_message<W: std::io::Write>(&self, out: &mut W) -> Result<()> {
        let line_ending = if self
            .conformance
            .contains(MessageConformance::NON_CANONICAL_LINE_ENDINGS)
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
            let info = Rfc2045Info::new(&self.headers);
            let ct = info.content_type.ok_or({
                MailParsingError::WriteMessageWtf(
                    "expected to have Content-Type when there are child parts",
                )
            })?;
            let boundary = ct.get("boundary").ok_or({
                MailParsingError::WriteMessageWtf("expected Content-Type to have a boundary")
            })?;
            out.write_all(self.intro.as_bytes())
                .map_err(|_| MailParsingError::WriteMessageIOError)?;
            for p in &self.parts {
                write!(out, "--{boundary}{line_ending}")
                    .map_err(|_| MailParsingError::WriteMessageIOError)?;
                p.write_message(out)?;
            }
            write!(out, "--{boundary}--{line_ending}")
                .map_err(|_| MailParsingError::WriteMessageIOError)?;
            out.write_all(self.outro.as_bytes())
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

    pub fn replace_text_body(&mut self, content_type: &str, content: &str) -> Result<()> {
        let mut new_part = Self::new_text(content_type, content)?;
        self.bytes = new_part.bytes;
        self.body_offset = new_part.body_offset;
        self.body_len = new_part.body_len;
        // Remove any rfc2047 headers that might reflect how the content
        // is encoded. Note that we preserve Content-Disposition as that
        // isn't related purely to the how the content is encoded
        self.headers.remove_all_named("Content-Type");
        self.headers.remove_all_named("Content-Transfer-Encoding");
        // And add any from the new part
        self.headers.append(&mut new_part.headers.headers);
        Ok(())
    }

    pub fn replace_binary_body(&mut self, content_type: &str, content: &[u8]) -> Result<()> {
        let mut new_part = Self::new_binary(content_type, content, None)?;
        self.bytes = new_part.bytes;
        self.body_offset = new_part.body_offset;
        self.body_len = new_part.body_len;
        // Remove any rfc2047 headers that might reflect how the content
        // is encoded. Note that we preserve Content-Disposition as that
        // isn't related purely to the how the content is encoded
        self.headers.remove_all_named("Content-Type");
        self.headers.remove_all_named("Content-Transfer-Encoding");
        // And add any from the new part
        self.headers.append(&mut new_part.headers.headers);
        Ok(())
    }

    pub fn new_no_transfer_encoding(content_type: &str, bytes: &[u8]) -> Result<Self> {
        if bytes.iter().any(|b| !b.is_ascii()) {
            return Err(MailParsingError::EightBit);
        }

        let mut headers = HeaderMap::default();

        let ct = MimeParameters::new(content_type);
        headers.set_content_type(ct)?;

        let bytes = String::from_utf8_lossy(bytes).to_string();
        let body_len = bytes.len();

        Ok(Self {
            bytes: bytes.into(),
            headers,
            body_offset: 0,
            body_len,
            conformance: MessageConformance::default(),
            parts: vec![],
            intro: "".into(),
            outro: "".into(),
        })
    }

    /// Constructs a new part with textual utf8 content.
    /// quoted-printable transfer encoding will be applied,
    /// unless it is smaller to represent the text in base64
    pub fn new_text(content_type: &str, content: &str) -> Result<Self> {
        // We'll probably use qp, so speculatively do the work
        let qp_encoded = quoted_printable::encode(content);

        let (mut encoded, encoding) = if qp_encoded == content.as_bytes() {
            (qp_encoded, None)
        } else if qp_encoded.len() <= BASE64_RFC2045.encode_len(content.len()) {
            (qp_encoded, Some("quoted-printable"))
        } else {
            // Turns out base64 will be smaller; perhaps the content
            // is dominated by non-ASCII text?
            (
                BASE64_RFC2045.encode(content.as_bytes()).into_bytes(),
                Some("base64"),
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
        headers.set_content_type(ct)?;

        if let Some(encoding) = encoding {
            headers.set_content_transfer_encoding(MimeParameters::new(encoding))?;
        }

        let body_len = encoded.len();
        let bytes =
            String::from_utf8(encoded).expect("transfer encoder to produce valid ASCII output");

        Ok(Self {
            bytes: bytes.into(),
            headers,
            body_offset: 0,
            body_len,
            conformance: MessageConformance::default(),
            parts: vec![],
            intro: "".into(),
            outro: "".into(),
        })
    }

    pub fn new_text_plain(content: &str) -> Result<Self> {
        Self::new_text("text/plain", content)
    }

    pub fn new_html(content: &str) -> Result<Self> {
        Self::new_text("text/html", content)
    }

    pub fn new_multipart(
        content_type: &str,
        parts: Vec<Self>,
        boundary: Option<&str>,
    ) -> Result<Self> {
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
        headers.set_content_type(ct)?;

        Ok(Self {
            bytes: "".into(),
            headers,
            body_offset: 0,
            body_len: 0,
            conformance: MessageConformance::default(),
            parts,
            intro: "".into(),
            outro: "".into(),
        })
    }

    pub fn new_binary(
        content_type: &str,
        content: &[u8],
        options: Option<&AttachmentOptions>,
    ) -> Result<Self> {
        let mut encoded = BASE64_RFC2045.encode(content);
        if !encoded.ends_with("\r\n") {
            encoded.push_str("\r\n");
        }
        let mut headers = HeaderMap::default();

        let mut ct = MimeParameters::new(content_type);

        if let Some(opts) = options {
            let mut cd = MimeParameters::new(if opts.inline { "inline" } else { "attachment" });
            if let Some(name) = &opts.file_name {
                cd.set("filename", name);
                let encoding = if name.chars().any(|c| !c.is_ascii()) {
                    MimeParameterEncoding::QuotedRfc2047
                } else {
                    MimeParameterEncoding::None
                };
                ct.set_with_encoding("name", name, encoding);
            }
            headers.set_content_disposition(cd)?;

            if let Some(id) = &opts.content_id {
                headers.set_content_id(MessageID(id.to_string()))?;
            }
        }

        headers.set_content_type(ct)?;
        headers.set_content_transfer_encoding(MimeParameters::new("base64"))?;

        let body_len = encoded.len();

        Ok(Self {
            bytes: encoded.into(),
            headers,
            body_offset: 0,
            body_len,
            conformance: MessageConformance::default(),
            parts: vec![],
            intro: "".into(),
            outro: "".into(),
        })
    }

    /// Returns a SimplifiedStructure representation of the mime tree,
    /// with the (probable) primary text/plain and text/html parts
    /// pulled out, and the remaining parts recorded as a flat
    /// attachments array
    pub fn simplified_structure(&'a self) -> Result<SimplifiedStructure<'a>> {
        let parts = self.simplified_structure_pointers()?;

        let mut text = None;
        let mut html = None;

        let headers = &self
            .resolve_ptr(parts.header_part)
            .expect("header part to always be valid")
            .headers;

        if let Some(p) = parts.text_part.and_then(|p| self.resolve_ptr(p)) {
            text = match p.body()? {
                DecodedBody::Text(t) => Some(t),
                DecodedBody::Binary(_) => {
                    return Err(MailParsingError::BodyParse(
                        "expected text/plain part to be text, but it is binary".to_string(),
                    ))
                }
            };
        }
        if let Some(p) = parts.html_part.and_then(|p| self.resolve_ptr(p)) {
            html = match p.body()? {
                DecodedBody::Text(t) => Some(t),
                DecodedBody::Binary(_) => {
                    return Err(MailParsingError::BodyParse(
                        "expected text/html part to be text, but it is binary".to_string(),
                    ))
                }
            };
        }

        let mut attachments = vec![];
        for ptr in parts.attachments {
            attachments.push(self.resolve_ptr(ptr).expect("pointer to be valid").clone());
        }

        Ok(SimplifiedStructure {
            text,
            html,
            headers,
            attachments,
        })
    }

    /// Resolve a PartPointer to the corresponding MimePart
    pub fn resolve_ptr(&self, ptr: PartPointer) -> Option<&Self> {
        let mut current = self;
        let mut cursor = ptr.0.as_slice();

        loop {
            match cursor.first() {
                Some(&idx) => {
                    current = current.parts.get(idx as usize)?;
                    cursor = &cursor[1..];
                }
                None => {
                    // We have completed the walk
                    return Some(current);
                }
            }
        }
    }

    /// Resolve a PartPointer to the corresponding MimePart, for mutable access
    pub fn resolve_ptr_mut(&mut self, ptr: PartPointer) -> Option<&mut Self> {
        let mut current = self;
        let mut cursor = ptr.0.as_slice();

        loop {
            match cursor.first() {
                Some(&idx) => {
                    current = current.parts.get_mut(idx as usize)?;
                    cursor = &cursor[1..];
                }
                None => {
                    // We have completed the walk
                    return Some(current);
                }
            }
        }
    }

    /// Returns a set of PartPointers that locate the (probable) primary
    /// text/plain and text/html parts, and the remaining parts recorded
    /// as a flat attachments array.  The resulting
    /// PartPointers can be resolved to their actual instances for both
    /// immutable and mutable operations via resolve_ptr and resolve_ptr_mut.
    pub fn simplified_structure_pointers(&self) -> Result<SimplifiedStructurePointers> {
        self.simplified_structure_pointers_impl(None)
    }

    fn simplified_structure_pointers_impl(
        &self,
        my_idx: Option<u8>,
    ) -> Result<SimplifiedStructurePointers> {
        let info = Rfc2045Info::new(&self.headers);
        let is_inline = info
            .attachment_options
            .as_ref()
            .map(|ao| ao.inline)
            .unwrap_or(true);

        if let Some(ct) = &info.content_type {
            if is_inline {
                if ct.value == "text/plain" {
                    return Ok(SimplifiedStructurePointers {
                        text_part: Some(PartPointer::root_or_nth(my_idx)),
                        html_part: None,
                        header_part: PartPointer::root_or_nth(my_idx),
                        attachments: vec![],
                    });
                }
                if ct.value == "text/html" {
                    return Ok(SimplifiedStructurePointers {
                        html_part: Some(PartPointer::root_or_nth(my_idx)),
                        text_part: None,
                        header_part: PartPointer::root_or_nth(my_idx),
                        attachments: vec![],
                    });
                }
            }

            if ct.value.starts_with("multipart/") {
                let mut text_part = None;
                let mut html_part = None;
                let mut attachments = vec![];

                for (i, p) in self.parts.iter().enumerate() {
                    let part_idx = i.try_into().map_err(|_| MailParsingError::TooManyParts)?;
                    if let Ok(mut s) = p.simplified_structure_pointers_impl(Some(part_idx)) {
                        if let Some(p) = s.text_part {
                            if text_part.is_none() {
                                text_part.replace(PartPointer::root_or_nth(my_idx).append(p));
                            } else {
                                attachments.push(p);
                            }
                        }
                        if let Some(p) = s.html_part {
                            if html_part.is_none() {
                                html_part.replace(PartPointer::root_or_nth(my_idx).append(p));
                            } else {
                                attachments.push(p);
                            }
                        }
                        attachments.append(&mut s.attachments);
                    }
                }

                return Ok(SimplifiedStructurePointers {
                    html_part,
                    text_part,
                    header_part: PartPointer::root_or_nth(my_idx),
                    attachments,
                });
            }

            return Ok(SimplifiedStructurePointers {
                html_part: None,
                text_part: None,
                header_part: PartPointer::root_or_nth(my_idx),
                attachments: vec![PartPointer::root_or_nth(my_idx)],
            });
        }

        // Assume text/plain content-type
        Ok(SimplifiedStructurePointers {
            text_part: Some(PartPointer::root_or_nth(my_idx)),
            html_part: None,
            header_part: PartPointer::root_or_nth(my_idx),
            attachments: vec![],
        })
    }
}

/// References the position of a MimePart by encoding the steps in
/// a tree walking operation. The encoding of PartPointer is a
/// sequence of integers that identify the index of a child part
/// by its level within the mime tree, selecting the current node
/// when no more indices remain. eg: `[]` indicates the
/// root part, while `[0]` is the 0th child of the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartPointer(Vec<u8>);

impl PartPointer {
    /// Construct a PartPointer that references the root node
    pub fn root() -> Self {
        Self(vec![])
    }

    /// Construct a PartPointer that references either the nth
    /// or the root node depending upon the passed parameter
    pub fn root_or_nth(n: Option<u8>) -> Self {
        match n {
            Some(n) => Self::nth(n),
            None => Self::root(),
        }
    }

    /// Construct a PartPointer that references the nth child
    pub fn nth(n: u8) -> Self {
        Self(vec![n])
    }

    /// Join other onto self, consuming self and producing
    /// a pointer that makes other relative to self
    pub fn append(mut self, mut other: Self) -> Self {
        self.0.append(&mut other.0);
        Self(self.0)
    }

    pub fn id_string(&self) -> String {
        let mut id = String::new();
        for p in &self.0 {
            if !id.is_empty() {
                id.push('.');
            }
            id.push_str(&p.to_string());
        }
        id
    }
}

#[derive(Debug, Clone)]
pub struct SimplifiedStructurePointers {
    /// The primary text/plain part
    pub text_part: Option<PartPointer>,
    /// The primary text/html part
    pub html_part: Option<PartPointer>,
    /// The "top level" set of headers for the message
    pub header_part: PartPointer,
    /// all other (terminal) parts are attachments
    pub attachments: Vec<PartPointer>,
}

#[derive(Debug, Clone)]
pub struct SimplifiedStructure<'a> {
    pub text: Option<SharedString<'a>>,
    pub html: Option<SharedString<'a>>,
    pub headers: &'a HeaderMap<'a>,
    pub attachments: Vec<MimePart<'a>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentOptions {
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub inline: bool,
    #[serde(default)]
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

impl<'a> DecodedBody<'a> {
    pub fn to_string_lossy(&'a self) -> Cow<'a, str> {
        match self {
            Self::Text(s) => Cow::Borrowed(s),
            Self::Binary(b) => String::from_utf8_lossy(b),
        }
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
Subject: hello there\r
From: Someone <someone@example.com>\r
\r
I am the body\r

"#
        );
    }

    #[test]
    fn mime_bogus_body() {
        let message = concat!(
            "Subject: hello there\n",
            "From: Someone <someone@example.com>\n",
            "Mime-Version: 1.0\n",
            "Content-Type: text/plain\n",
            "Content-Transfer-Encoding: base64\n",
            "\n",
            "hello\n"
        );

        let part = MimePart::parse(message).unwrap();
        assert_eq!(
            part.body().unwrap_err(),
            MailParsingError::BodyParse(
                "base64 decode: invalid length at 4 b='o' in hello\n".to_string()
            )
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
    fn replace_text_body() {
        let mut part = MimePart::new_text_plain("Hello 👻\r\n").unwrap();
        let encoded = part.to_message_string();
        k9::snapshot!(
            &encoded,
            r#"
Content-Type: text/plain;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: base64\r
\r
SGVsbG8g8J+Ruw0K\r

"#
        );

        part.replace_text_body("text/plain", "Hello 🚀\r\n")
            .unwrap();
        let encoded = part.to_message_string();
        k9::snapshot!(
            &encoded,
            r#"
Content-Type: text/plain;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: base64\r
\r
SGVsbG8g8J+agA0K\r

"#
        );
    }

    #[test]
    fn construct_1() {
        let input_text = "Well, hello there! This is the plaintext version, in utf-8. Here's a Euro: €, and here are some emoji 👻 🍉 💩 and this long should be long enough that we wrap it in the returned part, let's see how that turns out!\r\n";

        let part = MimePart::new_text_plain(input_text).unwrap();

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
        k9::snapshot!(
            parsed_part.simplified_structure_pointers(),
            "
Ok(
    SimplifiedStructurePointers {
        text_part: Some(
            PartPointer(
                [],
            ),
        ),
        html_part: None,
        header_part: PartPointer(
            [],
        ),
        attachments: [],
    },
)
"
        );
    }

    #[test]
    fn construct_2() {
        let msg = MimePart::new_multipart(
            "multipart/mixed",
            vec![
                MimePart::new_text_plain("plain text").unwrap(),
                MimePart::new_html("<b>rich</b> text").unwrap(),
                MimePart::new_binary(
                    "application/octet-stream",
                    &[0, 1, 2, 3],
                    Some(&AttachmentOptions {
                        file_name: Some("woot.bin".to_string()),
                        inline: false,
                        content_id: Some("woot.id".to_string()),
                    }),
                )
                .unwrap(),
            ],
            Some("my-boundary"),
        )
        .unwrap();
        k9::snapshot!(
            msg.to_message_string(),
            r#"
Content-Type: multipart/mixed;\r
\tboundary="my-boundary"\r
\r
--my-boundary\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
\r
plain text\r
--my-boundary\r
Content-Type: text/html;\r
\tcharset="us-ascii"\r
\r
<b>rich</b> text\r
--my-boundary\r
Content-Disposition: attachment;\r
\tfilename="woot.bin"\r
Content-ID: <woot.id>\r
Content-Type: application/octet-stream;\r
\tname="woot.bin"\r
Content-Transfer-Encoding: base64\r
\r
AAECAw==\r
--my-boundary--\r

"#
        );

        k9::snapshot!(
            msg.simplified_structure_pointers(),
            "
Ok(
    SimplifiedStructurePointers {
        text_part: Some(
            PartPointer(
                [
                    0,
                ],
            ),
        ),
        html_part: Some(
            PartPointer(
                [
                    1,
                ],
            ),
        ),
        header_part: PartPointer(
            [],
        ),
        attachments: [
            PartPointer(
                [
                    2,
                ],
            ),
        ],
    },
)
"
        );
    }

    #[test]
    fn attachment_name_order_prefers_content_disposition() {
        let message = concat!(
            "Content-Type: multipart/mixed;\r\n",
            "	boundary=\"woot\"\r\n",
            "\r\n",
            "--woot\r\n",
            "Content-Type: text/plain;\r\n",
            "	charset=\"us-ascii\"\r\n",
            "\r\n",
            "Hello, I am the main message content\r\n",
            "--woot\r\n",
            "Content-Disposition: attachment;\r\n",
            "	filename=cdname\r\n",
            "Content-Type: application/octet-stream;\r\n",
            "	name=ctname\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "u6o=\r\n",
            "--woot--\r\n"
        );
        let part = MimePart::parse(message).unwrap();
        let structure = part.simplified_structure().unwrap();

        k9::assert_equal!(
            structure.attachments[0].rfc2045_info().attachment_options,
            Some(AttachmentOptions {
                content_id: None,
                inline: false,
                file_name: Some("cdname".to_string()),
            })
        );
    }

    #[test]
    fn attachment_name_accepts_content_type_name() {
        let message = concat!(
            "Content-Type: multipart/mixed;\r\n",
            "	boundary=\"woot\"\r\n",
            "\r\n",
            "--woot\r\n",
            "Content-Type: text/plain;\r\n",
            "	charset=\"us-ascii\"\r\n",
            "\r\n",
            "Hello, I am the main message content\r\n",
            "--woot\r\n",
            "Content-Disposition: attachment\r\n",
            "Content-Type: application/octet-stream;\r\n",
            "	name=ctname\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "u6o=\r\n",
            "--woot--\r\n"
        );
        let part = MimePart::parse(message).unwrap();
        let structure = part.simplified_structure().unwrap();

        k9::assert_equal!(
            structure.attachments[0].rfc2045_info().attachment_options,
            Some(AttachmentOptions {
                content_id: None,
                inline: false,
                file_name: Some("ctname".to_string()),
            })
        );
    }

    #[test]
    fn funky_headers() {
        let message = concat!(
            "Subject\r\n",
            "Other:\r\n",
            "Content-Type: multipart/alternative; boundary=foobar\r\n",
            "Mime-Version: 1.0\r\n",
            "Date: Sun, 02 Oct 2016 07:06:22 -0700 (PDT)\r\n",
            "\r\n",
            "The body.\r\n"
        );

        let part = MimePart::parse(message).unwrap();
        assert!(part
            .conformance()
            .contains(MessageConformance::MISSING_COLON_VALUE));
    }

    /// This is a regression test for an issue where we'd interpret the
    /// binary bytes as default windows-1252 codepage charset, and mangle them.
    /// The high byte is sufficient to trigger the offending code prior
    /// to the fix
    #[test]
    fn rebuild_binary() {
        let expect = &[0, 1, 2, 3, 0xbe, 4, 5];
        let part = MimePart::new_binary("applicat/octet-stream", expect, None).unwrap();

        let rebuilt = part.rebuild().unwrap();
        let body = rebuilt.body().unwrap();

        assert_eq!(body, DecodedBody::Binary(expect.to_vec()));
    }

    /// Validate that we don't lose supplemental mime parameters like:
    /// `Content-Type: text/calendar; method=REQUEST`
    #[test]
    fn rebuild_invitation() {
        let message = concat!(
            "Subject: Test for events 2\r\n",
            "Content-Type: multipart/mixed;\r\n",
            " boundary=8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3\r\n",
            "\r\n",
            "--8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3\r\n",
            "Content-Type: multipart/alternative;\r\n",
            " boundary=a4e0aff9e05c7d94e2e13bd5590302f7802daac1e952c065207790d15a9f\r\n",
            "\r\n",
            "--a4e0aff9e05c7d94e2e13bd5590302f7802daac1e952c065207790d15a9f\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n",
            "Content-Type: text/plain; charset=UTF-8\r\n",
            "\r\n",
            "This is a test for calendar event invitation\r\n",
            "--a4e0aff9e05c7d94e2e13bd5590302f7802daac1e952c065207790d15a9f\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n",
            "Content-Type: text/html; charset=UTF-8\r\n",
            "\r\n",
            "<p>This is a test for calendar event invitation</p>\r\n",
            "--a4e0aff9e05c7d94e2e13bd5590302f7802daac1e952c065207790d15a9f--\r\n",
            "\r\n",
            "--8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3\r\n",
            "Content-Disposition: inline; name=\"Invitation.ics\"\r\n",
            "Content-Type: text/calendar; method=REQUEST; name=\"Invitation.ics\"\r\n",
            "\r\n",
            "Invitation\r\n",
            "--8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3\r\n",
            "Content-Disposition: attachment; filename=\"event.ics\"\r\n",
            "Content-Type: application/ics\r\n",
            "\r\n",
            "Event\r\n",
            "--8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3--\r\n",
            "\r\n"
        );

        let part = MimePart::parse(message).unwrap();
        let rebuilt = part.rebuild().unwrap();

        k9::snapshot!(
            rebuilt.to_message_string(),
            r#"
Content-Type: multipart/mixed;\r
\tboundary="8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3"\r
Subject: Test for events 2\r
\r
--8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3\r
Content-Type: multipart/alternative;\r
\tboundary="a4e0aff9e05c7d94e2e13bd5590302f7802daac1e952c065207790d15a9f"\r
\r
--a4e0aff9e05c7d94e2e13bd5590302f7802daac1e952c065207790d15a9f\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
\r
This is a test for calendar event invitation\r
--a4e0aff9e05c7d94e2e13bd5590302f7802daac1e952c065207790d15a9f\r
Content-Type: text/html;\r
\tcharset="us-ascii"\r
\r
<p>This is a test for calendar event invitation</p>\r
--a4e0aff9e05c7d94e2e13bd5590302f7802daac1e952c065207790d15a9f--\r
--8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3\r
Content-Type: text/calendar;\r
\tcharset="us-ascii";\r
\tmethod="REQUEST";\r
\tname="Invitation.ics"\r
\r
Invitation\r
--8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3\r
Content-Disposition: attachment;\r
\tfilename="event.ics"\r
Content-Type: application/ics;\r
\tname="event.ics"\r
Content-Transfer-Encoding: base64\r
\r
RXZlbnQNCg==\r
--8a54d64d7ad7c04a084478052b36cbe1609b33bf3a41203aaee8dd642cd3--\r

"#
        );
    }
}
