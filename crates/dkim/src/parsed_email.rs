use mailparse::MailHeader;
use memchr::memmem::Finder;
use once_cell::sync::Lazy;

static CRLFCRLF: Lazy<Finder> = Lazy::new(|| memchr::memmem::Finder::new("\r\n\r\n"));

pub enum ParsedEmail<'a> {
    FullyParsed(mailparse::ParsedMail<'a>),
    HeaderOnlyParse {
        headers: Vec<MailHeader<'a>>,
        body_bytes: &'a [u8],
    },
}

impl<'a> From<mailparse::ParsedMail<'a>> for ParsedEmail<'a> {
    fn from(mail: mailparse::ParsedMail<'a>) -> Self {
        Self::FullyParsed(mail)
    }
}

impl<'a> ParsedEmail<'a> {
    pub fn parse_bytes(bytes: &'a [u8]) -> Option<Self> {
        if CRLFCRLF.find(bytes).is_none() {
            // Canonical line endings are required, but are missing
            return None;
        }

        let (headers, offset) = mailparse::parse_headers(bytes).ok()?;
        Some(Self::HeaderOnlyParse {
            headers,
            body_bytes: &bytes[offset..],
        })
    }

    pub fn get_body_bytes(&self) -> &[u8] {
        match self {
            Self::FullyParsed(email) => CRLFCRLF
                .find(email.raw_bytes)
                .map(|idx| &email.raw_bytes[idx + 4..])
                .unwrap_or(b""),
            Self::HeaderOnlyParse { body_bytes, .. } => body_bytes,
        }
    }

    pub fn get_headers(&self) -> &[MailHeader<'a>] {
        match self {
            Self::FullyParsed(email) => &email.headers,
            Self::HeaderOnlyParse { headers, .. } => headers,
        }
    }
}
