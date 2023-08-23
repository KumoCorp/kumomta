use mailparsing::{
    Header, HeaderConformance, HeaderMap, HeaderParseResult, MimePart, SharedString,
};

pub enum ParsedEmail<'a> {
    FullyParsed(MimePart<'a>),
    HeaderOnlyParse {
        parsed: HeaderParseResult<'a>,
        bytes: SharedString<'a>,
    },
}

impl<'a> From<MimePart<'a>> for ParsedEmail<'a> {
    fn from(mail: MimePart<'a>) -> Self {
        Self::FullyParsed(mail)
    }
}

impl<'a> ParsedEmail<'a> {
    pub fn parse<S: Into<SharedString<'a>>>(bytes: S) -> Option<Self> {
        let bytes: SharedString = bytes.into();
        let parsed = Header::parse_headers(bytes.clone()).ok()?;
        if parsed
            .overall_conformance
            .contains(HeaderConformance::NON_CANONICAL_LINE_ENDINGS)
        {
            // Canonical line endings are required, but are missing
            return None;
        }
        Some(Self::HeaderOnlyParse { parsed, bytes })
    }

    pub fn get_body(&'a self) -> SharedString<'a> {
        match self {
            Self::FullyParsed(email) => email.raw_body(),
            Self::HeaderOnlyParse { bytes, parsed } => bytes.slice(parsed.body_offset..bytes.len()),
        }
    }

    pub fn get_headers(&'a self) -> &HeaderMap<'a> {
        match self {
            Self::FullyParsed(email) => &email.headers(),
            Self::HeaderOnlyParse { parsed, .. } => &parsed.headers,
        }
    }
}
