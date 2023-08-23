use crate::DKIMError;
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

impl<'a> TryFrom<MimePart<'a>> for ParsedEmail<'a> {
    type Error = DKIMError;
    fn try_from(mail: MimePart<'a>) -> Result<Self, DKIMError> {
        if mail
            .header_conformance()
            .contains(HeaderConformance::NON_CANONICAL_LINE_ENDINGS)
        {
            return Err(DKIMError::CanonicalLineEndingsRequired);
        }
        Ok(Self::FullyParsed(mail))
    }
}

impl<'a> ParsedEmail<'a> {
    pub fn parse<S: Into<SharedString<'a>>>(bytes: S) -> Result<Self, DKIMError> {
        let bytes: SharedString = bytes.into();
        let parsed = Header::parse_headers(bytes.clone())?;
        if parsed
            .overall_conformance
            .contains(HeaderConformance::NON_CANONICAL_LINE_ENDINGS)
        {
            return Err(DKIMError::CanonicalLineEndingsRequired);
        }
        Ok(Self::HeaderOnlyParse { parsed, bytes })
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
