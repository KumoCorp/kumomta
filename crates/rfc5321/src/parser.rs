use crate::client_types::SmtpClientTimeouts;
#[cfg(test)]
use bstr::BStr;
use bstr::{BString, ByteSlice};
use nom::branch::alt;
use nom::bytes::complete::{take_while1, take_while_m_n};
use nom::combinator::{all_consuming, map, map_res, opt, recognize};
use nom::error::context;
use nom::multi::{many0, many1};
use nom::sequence::pair;
use nom::Parser;
use nom_utils::{
    domain_name, explain_nom, ipv4_address, ipv6_address, make_span, tag, tag_no_case,
    utf8_non_ascii, DomainString, IResult, Span,
};
use pastey::paste;
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandVerb {
    Ehlo,
    Helo,
    Lhlo,
    Mail,
    Rcpt,
    Data,
    Rset,
    Quit,
    Vrfy,
    Expn,
    Help,
    Noop,
    StartTls,
    Auth,
    XClient,
    Unknown(BString),
}

/// Domain part of a mailbox in a MAIL FROM address.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Domain {
    /// A valid DNS domain name
    DomainName(DomainString),
    /// An IPv4 address literal, e.g. from `[10.0.0.1]`
    V4(Ipv4Addr),
    /// An IPv6 address literal, e.g. from `[IPv6:::1]`
    V6(Ipv6Addr),
    /// A general/tagged address literal, e.g. from `[future:something]`.
    /// Stores the original `"tag:literal"` string; split on the first `:`
    /// when the tag or literal parts are needed individually.
    Tagged(String),
}

impl std::fmt::Debug for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Domain::DomainName(s) => write!(f, "{}", s),
            Domain::V4(ip) => write!(f, "[{}]", ip),
            Domain::V6(ip) => write!(f, "[IPv6:{}]", ip),
            Domain::Tagged(s) => write!(f, "[{}]", s),
        }
    }
}

impl std::fmt::Display for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Domain::DomainName(s) => write!(f, "{}", s),
            Domain::V4(ip) => write!(f, "[{}]", ip),
            Domain::V6(ip) => write!(f, "[IPv6:{}]", ip),
            Domain::Tagged(s) => write!(f, "[{}]", s),
        }
    }
}

impl Domain {
    /// Returns true if the wire representation of this domain is pure ASCII.
    ///
    /// `DomainName` is always normalized to ASCII punycode on the wire, so it
    /// is always considered ASCII here.  IP address literals are inherently
    /// ASCII.  Tagged literals are checked character-by-character.
    pub fn is_ascii(&self) -> bool {
        match self {
            Domain::DomainName(_) | Domain::V4(_) | Domain::V6(_) => true,
            Domain::Tagged(s) => s.is_ascii(),
        }
    }

}

/// An email mailbox: `local-part "@" domain`
#[derive(Clone, Debug)]
pub struct Mailbox {
    pub(crate) local_part: String,
    pub domain: Domain,
}

impl PartialEq for Mailbox {
    fn eq(&self, other: &Self) -> bool {
        self.local_part() == other.local_part() && self.domain == other.domain
    }
}

impl Eq for Mailbox {}

impl Mailbox {
    /// Returns true if both the local part and domain are pure ASCII.
    pub fn is_ascii(&self) -> bool {
        self.local_part.is_ascii() && self.domain.is_ascii()
    }

    /// Returns the normalized local part.
    /// Normalization removes any quoting from the local part,
    /// so that `"\f\o\o"` and `"foo"` will both be returned
    /// as `foo` and will compare as equal.
    /// Any byte sequences that are invalid UTF-8 will be
    /// replaced with the unicode replacement character.
    pub fn local_part(&self) -> Cow<'_, str> {
        // Check if the local_part is a quoted string
        if self.local_part.starts_with('"') {
            // Quoted string - need to unquote
            let mut result = String::new();
            let mut chars = self.local_part.chars();
            chars.next(); // skip initial quote
            while let Some(c) = chars.next() {
                match c {
                    '\\' => match chars.next() {
                        Some(c) => {
                            result.push(c);
                        }
                        None => {
                            result.push('\\');
                        }
                    },
                    '"' => {
                        // Probably the final closing quote.
                        // Should be impossible/illegal otherwise
                        continue;
                    }
                    c => {
                        result.push(c);
                    }
                }
            }
            Cow::Owned(result)
        } else {
            Cow::Borrowed(self.local_part.as_str())
        }
    }
}

#[cfg(test)]
mod mailbox_tests {
    use super::*;

    #[test]
    fn test_mailbox_local_part_normalized() {
        // Test that local_part() normalizes quoted strings
        let mb1 = Mailbox {
            local_part: String::from("foo"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let mb2 = Mailbox {
            local_part: String::from("\"foo\""),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let mb3 = Mailbox {
            local_part: String::from("\"f\\oo\""),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };

        // All three should normalize to "foo"
        k9::assert_equal!(mb1.local_part(), "foo");
        k9::assert_equal!(mb2.local_part(), "foo");
        k9::assert_equal!(mb3.local_part(), "foo");
    }

    #[test]
    fn test_mailbox_local_part_eq_normalized() {
        // Test that Mailbox equality uses normalized local_part
        let mb1 = Mailbox {
            local_part: String::from("foo"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let mb2 = Mailbox {
            local_part: String::from("\"foo\""),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let mb3 = Mailbox {
            local_part: String::from("\"f\\oo\""),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };

        // All three should be equal due to normalized local_part
        k9::assert_equal!(mb1, mb2);
        k9::assert_equal!(mb2, mb3);
        k9::assert_equal!(mb1, mb3);
    }

    #[test]
    fn test_mailbox_local_part_unquoted_borrowed() {
        // Test that unquoted valid UTF-8 returns Cow::Borrowed
        let mb = Mailbox {
            local_part: String::from("foo"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };

        let local_part = mb.local_part();
        match local_part {
            Cow::Borrowed(_) => {}
            Cow::Owned(_) => panic!("Expected Cow::Borrowed for unquoted valid UTF-8"),
        }
    }

    #[test]
    fn test_mailbox_local_part_quoted_unquoted_borrowed() {
        // Test that quoted valid UTF-8 returns Cow::Owned (needs unquoting)
        let mb = Mailbox {
            local_part: String::from("\"foo\""),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };

        let local_part = mb.local_part();
        match local_part {
            Cow::Borrowed(_) => panic!("Expected Cow::Owned for quoted string"),
            Cow::Owned(s) => {
                k9::assert_equal!(s, "foo");
            }
        }
    }
}

impl Hash for Mailbox {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.local_part().hash(state);
        self.domain.hash(state);
    }
}

/// A parsed email path: optional source route (at-domain-list) plus mailbox.
///
/// Per RFC 5321 §4.1.2, the source route (at-domain-list) MUST be accepted
/// when parsing, SHOULD NOT be generated when encoding, and SHOULD be ignored.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MailPath {
    /// Optional source route: list of domains (without the `@` prefix).
    pub at_domain_list: Vec<String>,
    /// The final mailbox (local-part@domain).
    pub mailbox: Mailbox,
}

impl std::fmt::Debug for MailPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut local_part = Vec::new();

        // Add source route if present
        if !self.at_domain_list.is_empty() {
            for (i, domain) in self.at_domain_list.iter().enumerate() {
                if i > 0 {
                    local_part.push(b',');
                }
                local_part.push(b'@'); // RFC 5321 source route has @ prefix
                local_part.extend_from_slice(domain.as_bytes());
            }
            local_part.push(b':');
        }

        // Add local-part
        local_part.extend_from_slice(self.mailbox.local_part.as_bytes());

        // Format as MailPath("local_part@domain") with proper escaping using escape_bytes
        write!(
            f,
            "MailPath(\"{}@{:?}\")",
            local_part.escape_bytes(),
            self.mailbox.domain
        )
    }
}

impl std::fmt::Display for MailPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.at_domain_list.is_empty() {
            for (i, domain) in self.at_domain_list.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "@{domain}")?;
            }
            f.write_str(":")?;
        }
        write!(f, "{}@{}", self.mailbox.local_part, self.mailbox.domain)
    }
}

impl MailPath {
    /// Returns true if the mailbox address is pure ASCII.
    ///
    /// The source route (`at_domain_list`) is intentionally ignored, matching
    /// the old parser behaviour (RFC 5321 says source routes SHOULD be ignored).
    pub fn is_ascii(&self) -> bool {
        self.mailbox.is_ascii()
    }
}

/// The reverse path (sender) for a MAIL FROM command
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReversePath {
    /// A mailbox path
    Path(MailPath),
    /// Null sender: `MAIL FROM:<>`
    NullSender,
}

impl ReversePath {
    /// Returns true if the address is pure ASCII.
    pub fn is_ascii(&self) -> bool {
        match self {
            Self::NullSender => true,
            Self::Path(p) => p.is_ascii(),
        }
    }
}

impl TryFrom<&str> for ReversePath {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        EnvelopeAddress::parse(s)?
            .try_into()
            .map_err(|e: &'static str| e.to_string())
    }
}

impl std::fmt::Display for ReversePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NullSender => Ok(()),
            Self::Path(p) => p.fmt(f),
        }
    }
}

/// The forward path (recipient) for a RCPT TO command
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ForwardPath {
    /// A mailbox path
    Path(MailPath),
    /// Postmaster: `RCPT TO:<Postmaster>`  (RFC 5321 §4.1.1.3)
    Postmaster,
}

impl ForwardPath {
    /// Returns true if the address is pure ASCII.
    pub fn is_ascii(&self) -> bool {
        match self {
            Self::Postmaster => true,
            Self::Path(p) => p.is_ascii(),
        }
    }
}

impl TryFrom<&str> for ForwardPath {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        EnvelopeAddress::parse(s)?
            .try_into()
            .map_err(|e: &'static str| e.to_string())
    }
}

impl std::fmt::Display for ForwardPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Postmaster => write!(f, "Postmaster"),
            Self::Path(p) => p.fmt(f),
        }
    }
}

/// An envelope address: either a path, null sender, or postmaster.
#[derive(Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EnvelopeAddress {
    /// Null sender: `<>`
    Null,
    /// Postmaster: `<Postmaster>` or `Postmaster`
    Postmaster,
    /// A path: `<path>` or bare path
    Path(MailPath),
}

impl std::fmt::Display for EnvelopeAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvelopeAddress::Null => write!(f, ""),
            EnvelopeAddress::Postmaster => write!(f, "Postmaster"),
            EnvelopeAddress::Path(path) => path.fmt(f),
        }
    }
}

impl std::fmt::Debug for EnvelopeAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{}>", self)
    }
}

impl From<MailPath> for EnvelopeAddress {
    fn from(path: MailPath) -> Self {
        EnvelopeAddress::Path(path)
    }
}

impl TryFrom<String> for EnvelopeAddress {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        EnvelopeAddress::parse(&s)
    }
}

impl From<EnvelopeAddress> for String {
    fn from(addr: EnvelopeAddress) -> Self {
        addr.to_string()
    }
}

impl std::str::FromStr for EnvelopeAddress {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EnvelopeAddress::parse(s)
    }
}

impl From<ForwardPath> for EnvelopeAddress {
    fn from(fp: ForwardPath) -> Self {
        match fp {
            ForwardPath::Postmaster => EnvelopeAddress::Postmaster,
            ForwardPath::Path(p) => EnvelopeAddress::Path(p),
        }
    }
}

impl TryFrom<ReversePath> for EnvelopeAddress {
    type Error = &'static str;
    fn try_from(rp: ReversePath) -> Result<Self, Self::Error> {
        match rp {
            ReversePath::NullSender => Ok(EnvelopeAddress::Null),
            ReversePath::Path(p) => Ok(EnvelopeAddress::Path(p)),
        }
    }
}

impl EnvelopeAddress {
    /// Parse an envelope address from a string.
    ///
    /// Accepts either forward or reverse path syntax, with or without angle brackets.
    pub fn parse(input: &str) -> Result<EnvelopeAddress, String> {
        let input = make_span(input.as_bytes());
        let (_, result) = all_consuming(alt((
            map(tag_no_case("<>"), |_| EnvelopeAddress::Null),
            map(tag_no_case("<Postmaster>"), |_| EnvelopeAddress::Postmaster),
            map(tag_no_case("Postmaster"), |_| EnvelopeAddress::Postmaster),
            map(path, EnvelopeAddress::Path),
            map(mailbox, EnvelopeAddress::from),
        )))
        .parse(input)
        .map_err(|e| explain_nom(input, e))?;
        Ok(result)
    }
}

impl From<MailPath> for ReversePath {
    fn from(path: MailPath) -> Self {
        ReversePath::Path(path)
    }
}

impl From<MailPath> for ForwardPath {
    fn from(path: MailPath) -> Self {
        ForwardPath::Path(path)
    }
}

impl TryFrom<ReversePath> for MailPath {
    type Error = &'static str;

    fn try_from(path: ReversePath) -> Result<Self, Self::Error> {
        match path {
            ReversePath::Path(mailpath) => Ok(mailpath),
            ReversePath::NullSender => Err("Cannot convert NullSender to MailPath"),
        }
    }
}

impl TryFrom<ForwardPath> for MailPath {
    type Error = &'static str;

    fn try_from(path: ForwardPath) -> Result<Self, Self::Error> {
        match path {
            ForwardPath::Path(mailpath) => Ok(mailpath),
            ForwardPath::Postmaster => Err("Cannot convert Postmaster to MailPath"),
        }
    }
}

// ============================================================================
// Conversions from Mailbox
// ============================================================================

/// Infallible conversion: wraps a Mailbox in a MailPath with no source route.
impl From<Mailbox> for MailPath {
    fn from(mailbox: Mailbox) -> Self {
        MailPath {
            at_domain_list: vec![],
            mailbox,
        }
    }
}

/// Infallible conversion: wraps a Mailbox in an EnvelopeAddress::Path.
impl From<Mailbox> for EnvelopeAddress {
    fn from(mailbox: Mailbox) -> Self {
        EnvelopeAddress::Path(MailPath {
            at_domain_list: vec![],
            mailbox,
        })
    }
}

/// Infallible conversion: wraps a Mailbox in a ReversePath::Path.
impl From<Mailbox> for ReversePath {
    fn from(mailbox: Mailbox) -> Self {
        ReversePath::Path(MailPath {
            at_domain_list: vec![],
            mailbox,
        })
    }
}

/// Infallible conversion: wraps a Mailbox in a ForwardPath::Path.
impl From<Mailbox> for ForwardPath {
    fn from(mailbox: Mailbox) -> Self {
        ForwardPath::Path(MailPath {
            at_domain_list: vec![],
            mailbox,
        })
    }
}

// ============================================================================
// Fallible conversions to Mailbox
// ============================================================================

impl TryFrom<EnvelopeAddress> for Mailbox {
    type Error = &'static str;

    fn try_from(addr: EnvelopeAddress) -> Result<Self, Self::Error> {
        match addr {
            EnvelopeAddress::Path(path) => Ok(path.mailbox),
            EnvelopeAddress::Null => Err("Cannot convert Null to Mailbox"),
            EnvelopeAddress::Postmaster => Err("Cannot convert Postmaster to Mailbox"),
        }
    }
}

impl TryFrom<ReversePath> for Mailbox {
    type Error = &'static str;

    fn try_from(path: ReversePath) -> Result<Self, Self::Error> {
        match path {
            ReversePath::Path(path) => Ok(path.mailbox),
            ReversePath::NullSender => Err("Cannot convert NullSender to Mailbox"),
        }
    }
}

impl TryFrom<ForwardPath> for Mailbox {
    type Error = &'static str;

    fn try_from(path: ForwardPath) -> Result<Self, Self::Error> {
        match path {
            ForwardPath::Path(path) => Ok(path.mailbox),
            ForwardPath::Postmaster => Err("Cannot convert Postmaster to Mailbox"),
        }
    }
}

// ============================================================================
// Fallible conversions to MailPath
// ============================================================================

impl TryFrom<EnvelopeAddress> for MailPath {
    type Error = &'static str;

    fn try_from(addr: EnvelopeAddress) -> Result<Self, Self::Error> {
        match addr {
            EnvelopeAddress::Path(path) => Ok(path),
            EnvelopeAddress::Null => Err("Cannot convert Null to MailPath"),
            EnvelopeAddress::Postmaster => Err("Cannot convert Postmaster to MailPath"),
        }
    }
}

impl TryFrom<ReversePath> for ForwardPath {
    type Error = &'static str;

    fn try_from(path: ReversePath) -> Result<Self, Self::Error> {
        match path {
            ReversePath::Path(mailpath) => Ok(ForwardPath::Path(mailpath)),
            ReversePath::NullSender => Err("Cannot convert NullSender to ForwardPath"),
        }
    }
}

impl TryFrom<ForwardPath> for ReversePath {
    type Error = &'static str;

    fn try_from(path: ForwardPath) -> Result<Self, Self::Error> {
        match path {
            ForwardPath::Path(mailpath) => Ok(ReversePath::Path(mailpath)),
            ForwardPath::Postmaster => Err("Cannot convert Postmaster to ReversePath"),
        }
    }
}

impl TryFrom<EnvelopeAddress> for ReversePath {
    type Error = &'static str;

    fn try_from(addr: EnvelopeAddress) -> Result<Self, Self::Error> {
        match addr {
            EnvelopeAddress::Path(path) => Ok(ReversePath::Path(path)),
            EnvelopeAddress::Null => Ok(ReversePath::NullSender),
            EnvelopeAddress::Postmaster => Err("Cannot convert Postmaster to ReversePath"),
        }
    }
}

impl TryFrom<EnvelopeAddress> for ForwardPath {
    type Error = &'static str;

    fn try_from(addr: EnvelopeAddress) -> Result<Self, Self::Error> {
        match addr {
            EnvelopeAddress::Path(path) => Ok(ForwardPath::Path(path)),
            EnvelopeAddress::Null => Err("Cannot convert Null to ForwardPath"),
            EnvelopeAddress::Postmaster => Ok(ForwardPath::Postmaster),
        }
    }
}

/// An ESMTP parameter: `name["=" value]`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EsmtpParameter {
    pub name: String,
    pub value: Option<String>,
}

/// A single XCLIENT parameter: `name=xtext-value`.
/// The `value` field stores the **xtext-decoded** string; the wire form
/// uses xtext encoding where non-printable bytes appear as `+XX` hex pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XClientParameter {
    pub name: String,
    pub value: String,
}

impl XClientParameter {
    /// Returns true if the parameter name matches the given name (case-insensitive).
    pub fn is_name(&self, name: impl AsRef<str>) -> bool {
        self.name.eq_ignore_ascii_case(name.as_ref())
    }

    /// Parse the parameter value as type T.
    ///
    /// Converts the value to a string and then parses it as T.
    pub fn parse<T>(&self) -> Result<T, String>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        let parsed: Result<T, T::Err> = self.value.parse();
        parsed.map_err(|e| e.to_string())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Command {
    Ehlo(Domain),
    Helo(Domain),
    Lhlo(Domain),
    Noop(Option<String>),
    Help(Option<String>),
    Vrfy(Option<String>),
    Expn(Option<String>),
    Data,
    /// The end-of-data terminator sent after the message body: `".\r\n"`.
    ///
    /// This variant is never produced by the parser — it is constructed
    /// programmatically by SMTP client code and serialized via
    /// [`Command::encode`] when the client needs to signal the end of the
    /// DATA content stream (RFC 5321 §4.5.2).
    DataDot,
    Rset,
    Quit,
    StartTls,
    MailFrom {
        address: ReversePath,
        parameters: Vec<EsmtpParameter>,
    },
    RcptTo {
        address: ForwardPath,
        parameters: Vec<EsmtpParameter>,
    },
    Auth {
        sasl_mech: String,
        initial_response: Option<String>,
    },
    XClient(Vec<XClientParameter>),
    Unknown(BString),
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use Command::encode to get the wire format, then escape_bytes for Debug output
        let encoded = self.encode();
        write!(f, "Command(\"{}\")", encoded.escape_bytes())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum MaybePartialCommand {
    Full(Command),
    Partial {
        verb: CommandVerb,
        remainder: BString,
    },
}

impl std::fmt::Debug for MaybePartialCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaybePartialCommand::Full(cmd) => write!(f, "Full({cmd:?})"),
            MaybePartialCommand::Partial { verb, remainder } => {
                write!(
                    f,
                    "Partial {{ verb: {verb:?}, remainder: {:?} }}",
                    remainder.escape_bytes()
                )
            }
        }
    }
}

macro_rules! parse_single {
    ($func_name:ident, $token:literal, $verb:ident) => {
        fn $func_name(input: Span) -> IResult<Span, MaybePartialCommand> {
            context(
                $token,
                alt((
                    map(
                        all_consuming((tag_no_case($token), wsp, anything)),
                        |(_cmd, _space, remainder)| MaybePartialCommand::Partial {
                            verb: CommandVerb::$verb,
                            remainder: (*remainder).into(),
                        },
                    ),
                    map(all_consuming(tag_no_case($token)), |_| {
                        MaybePartialCommand::Full(Command::$verb)
                    }),
                )),
            )
            .parse(input)
        }

        paste! {
            #[cfg(test)]
            #[test]
            fn [<test_ $func_name>]() {
                k9::assert_equal!(
                    unwrapper(Command::parse($token)),
                    MaybePartialCommand::Full(Command::$verb)
                );
                k9::assert_equal!(
                    unwrapper(Command::parse($token.to_lowercase())),
                    MaybePartialCommand::Full(Command::$verb)
                );
                k9::assert_equal!(
                    unwrapper(Command::parse(format!("{} trailing garbage", $token))),
                    MaybePartialCommand::Partial {
                        verb: CommandVerb::$verb,
                        remainder:"trailing garbage".into()
                    }
                );
            }
        }
    };
}

macro_rules! parse_opt_arg {
    ($func_name:ident, $token:literal, $verb:ident) => {
        fn $func_name(input: Span) -> IResult<Span, MaybePartialCommand> {
            context(
                $token,
                alt((
                    map(
                        all_consuming((tag_no_case($token), wsp, string)),
                        |(_cmd, _space, param)| match String::from_utf8(param.fragment().to_vec()) {
                            Ok(s) => MaybePartialCommand::Full(Command::$verb(Some(s))),
                            Err(_) => MaybePartialCommand::Partial {
                                verb: CommandVerb::$verb,
                                remainder: BString::default(),
                            },
                        },
                    ),
                    map(
                        all_consuming((tag_no_case($token), wsp, anything)),
                        |(_cmd, _space, remainder)| MaybePartialCommand::Partial {
                            verb: CommandVerb::$verb,
                            remainder: (*remainder).into(),
                        },
                    ),
                    map(all_consuming(tag_no_case($token)), |_| {
                        MaybePartialCommand::Full(Command::$verb(None))
                    }),
                )),
            )
            .parse(input)
        }

        paste! {
            #[cfg(test)]
            #[test]
            fn [<test_ $func_name>]() {
                k9::assert_equal!(
                    unwrapper(Command::parse($token)),
                    MaybePartialCommand::Full(Command::$verb(None)),
                    "full no param"
                );
                k9::assert_equal!(
                    unwrapper(Command::parse($token.to_lowercase())),
                    MaybePartialCommand::Full(Command::$verb(None)),
                    "full no param, different case"
                );
                k9::assert_equal!(
                    unwrapper(Command::parse(format!("{} parameter", $token))),
                    MaybePartialCommand::Full(Command::$verb(Some("parameter".into()))),
                    "full with param"
                );
                k9::assert_equal!(
                    unwrapper(Command::parse(format!("{} trailing garbage", $token))),
                    MaybePartialCommand::Partial {
                        verb: CommandVerb::$verb,
                        remainder:"trailing garbage".into()
                    },
                    "should have partial"
                );
            }
        }
    };
}

/// Helper that does an unwrap, but rather than print the error string
/// with escapes, uses its Display impl. This makes it easier to see
/// what the error message is, because the error in this context is
/// typically a nom_utils error which is multi-line and uses a caret
/// to point to the appropriate column in the input.
#[cfg(test)]
fn unwrapper<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
    match result {
        Ok(r) => r,
        Err(err) => panic!("{err}"),
    }
}

parse_opt_arg!(parse_noop, "NOOP", Noop);
parse_opt_arg!(parse_help, "HELP", Help);
parse_opt_arg!(parse_vrfy, "VRFY", Vrfy);
parse_opt_arg!(parse_expn, "EXPN", Expn);
parse_single!(parse_data, "DATA", Data);
parse_single!(parse_rset, "RSET", Rset);
parse_single!(parse_quit, "QUIT", Quit);
parse_single!(parse_starttls, "STARTTLS", StartTls);

fn parse_with<'a, R, F>(text: &'a [u8], parser: F) -> Result<R, String>
where
    F: Fn(Span<'a>) -> IResult<'a, Span<'a>, R>,
{
    let input = make_span(text);
    let (_, result) = all_consuming(parser)
        .parse(input)
        .map_err(|err| explain_nom(input, err))?;
    Ok(result)
}

impl Command {
    pub fn parse(input: impl AsRef<[u8]>) -> Result<MaybePartialCommand, String> {
        // Strip a trailing CRLF (or bare LF) so that both wire-format input
        // (with CRLF terminator, as produced by encode()) and bare command
        // strings (as used in tests and interactive contexts) are accepted.
        let bytes = input.as_ref();
        let bytes = bytes
            .strip_suffix(b"\r\n")
            .or_else(|| bytes.strip_suffix(b"\n"))
            .unwrap_or(bytes);
        parse_with(bytes, Self::parse_span)
    }

    fn parse_span(input: Span) -> IResult<Span, MaybePartialCommand> {
        context(
            "command-verb",
            alt((
                parse_ehlo,
                parse_helo,
                parse_lhlo,
                parse_help,
                parse_noop,
                parse_vrfy,
                parse_expn,
                parse_data,
                parse_rset,
                parse_quit,
                parse_starttls,
                parse_mail_from,
                parse_rcpt_to,
                parse_auth,
                parse_xclient,
                Self::parse_unknown,
            )),
        )
        .parse(input)
    }

    /// Re-encode the command as a single line of text ready to send on the
    /// wire, including the trailing `\r\n`.
    ///
    /// The returned `BString` can be fed back to [`Command::parse`] to
    /// recover the original `Command` value (round-trip stable), with the
    /// one intentional exception that `at_domain_list` source routes are not
    /// re-emitted (RFC 5321 says they SHOULD NOT be generated).
    pub fn encode(&self) -> BString {
        let mut buf: Vec<u8> = Vec::new();
        match self {
            Self::Ehlo(domain) => {
                buf.extend_from_slice(b"EHLO ");
                buf.extend_from_slice(encode_domain(domain).as_ref());
            }
            Self::Helo(domain) => {
                buf.extend_from_slice(b"HELO ");
                buf.extend_from_slice(encode_domain(domain).as_ref());
            }
            Self::Lhlo(domain) => {
                buf.extend_from_slice(b"LHLO ");
                buf.extend_from_slice(encode_domain(domain).as_ref());
            }
            Self::Noop(None) => buf.extend_from_slice(b"NOOP"),
            Self::Noop(Some(s)) => {
                buf.extend_from_slice(b"NOOP ");
                buf.extend_from_slice(s.as_bytes());
            }
            Self::Help(None) => buf.extend_from_slice(b"HELP"),
            Self::Help(Some(s)) => {
                buf.extend_from_slice(b"HELP ");
                buf.extend_from_slice(s.as_bytes());
            }
            Self::Vrfy(None) => buf.extend_from_slice(b"VRFY"),
            Self::Vrfy(Some(s)) => {
                buf.extend_from_slice(b"VRFY ");
                buf.extend_from_slice(s.as_bytes());
            }
            Self::Expn(None) => buf.extend_from_slice(b"EXPN"),
            Self::Expn(Some(s)) => {
                buf.extend_from_slice(b"EXPN ");
                buf.extend_from_slice(s.as_bytes());
            }
            Self::Data => buf.extend_from_slice(b"DATA"),
            // DataDot encodes as exactly ".\r\n" — return before the
            // unconditional CRLF that all other arms rely on below.
            Self::DataDot => return BString::from(".\r\n"),
            Self::Rset => buf.extend_from_slice(b"RSET"),
            Self::Quit => buf.extend_from_slice(b"QUIT"),
            Self::StartTls => buf.extend_from_slice(b"STARTTLS"),
            Self::MailFrom {
                address,
                parameters,
            } => {
                buf.extend_from_slice(b"MAIL FROM:<");
                buf.extend(encode_reverse_path(address));
                buf.push(b'>');
                buf.extend(encode_esmtp_params(parameters));
            }
            Self::RcptTo {
                address,
                parameters,
            } => {
                buf.extend_from_slice(b"RCPT TO:<");
                buf.extend(encode_forward_path(address));
                buf.push(b'>');
                buf.extend(encode_esmtp_params(parameters));
            }
            Self::Auth {
                sasl_mech,
                initial_response: None,
            } => {
                buf.extend_from_slice(b"AUTH ");
                buf.extend_from_slice(sasl_mech.as_bytes());
            }
            Self::Auth {
                sasl_mech,
                initial_response: Some(resp),
            } => {
                buf.extend_from_slice(b"AUTH ");
                buf.extend_from_slice(sasl_mech.as_bytes());
                buf.push(b' ');
                buf.extend_from_slice(resp.as_bytes());
            }
            Self::XClient(params) => {
                buf.extend_from_slice(b"XCLIENT");
                buf.extend(encode_xclient_params(params));
            }
            Self::Unknown(s) => {
                buf.extend_from_slice(s);
            }
        }
        buf.extend_from_slice(b"\r\n");
        BString::from(buf)
    }

    /// Timeout for reading the response to this command.
    pub fn client_timeout(&self, timeouts: &SmtpClientTimeouts) -> Duration {
        match self {
            Self::Helo(_) | Self::Ehlo(_) | Self::Lhlo(_) => timeouts.ehlo_timeout,
            Self::MailFrom { .. } => timeouts.mail_from_timeout,
            Self::RcptTo { .. } => timeouts.rcpt_to_timeout,
            Self::Data => timeouts.data_timeout,
            Self::DataDot => timeouts.data_dot_timeout,
            Self::Rset => timeouts.rset_timeout,
            Self::StartTls => timeouts.starttls_timeout,
            Self::Quit | Self::Vrfy(_) | Self::Expn(_) | Self::Help(_) | Self::Noop(_) => {
                timeouts.idle_timeout
            }
            Self::Auth { .. } => timeouts.auth_timeout,
            Self::XClient(_) => timeouts.auth_timeout, // FIXME: xclient specific timeout
            Self::Unknown(_) => timeouts.mail_from_timeout, // No good option for this TBH.
        }
    }

    /// Timeout for writing the request.
    pub fn client_timeout_request(&self, timeouts: &SmtpClientTimeouts) -> Duration {
        self.client_timeout(timeouts).min(Duration::from_secs(60))
    }

    fn parse_unknown(input: Span) -> IResult<Span, MaybePartialCommand> {
        context(
            "unknown-command",
            alt((
                map(
                    all_consuming(recognize((command_word, wsp, anything))),
                    |command| MaybePartialCommand::Full(Command::Unknown((*command).into())),
                ),
                map(all_consuming(command_word), |command| {
                    MaybePartialCommand::Full(Command::Unknown((*command).into()))
                }),
            )),
        )
        .parse(input)
    }
}

fn command_word(input: Span) -> IResult<Span, Span> {
    context(
        "command-word",
        take_while1(|c: u8| c.is_ascii_alphanumeric()),
    )
    .parse(input)
}

fn wsp(input: Span) -> IResult<Span, Span> {
    context("wsp", take_while1(|c| c == b' ' || c == b'\t')).parse(input)
}

fn anything(input: Span) -> IResult<Span, Span> {
    context("anything", take_while1(|_| true)).parse(input)
}

fn atext(input: Span) -> IResult<Span, Span> {
    recognize(alt((
        take_while_m_n(1, 1, |c: u8| {
            matches!(
                c,
                b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'/' | b'=' | b'?'
                    | b'^' | b'_' | b'`' | b'{' | b'|' | b'}' | b'~'
                    | b'A'..=b'Z'
                    | b'a'..=b'z'
                    | b'0'..=b'9'
            )
        }),
        utf8_non_ascii,
    )))
    .parse(input)
}

fn atom(input: Span) -> IResult<Span, Span> {
    context("atom", recognize(many1(atext))).parse(input)
}

fn quoted_string(input: Span) -> IResult<Span, Span> {
    context(
        "quoted-string",
        recognize((
            tag("\""),
            many0(alt((
                recognize(pair(
                    tag("\\"),
                    take_while_m_n(1, 1, |c: u8| c >= 0x20 && c <= 0x7e),
                )),
                take_while_m_n(1, 1, |c: u8| {
                    (c >= 0x20 && c <= 0x21) || (c >= 0x23 && c <= 0x5b) || (c >= 0x5d && c <= 0x7e)
                }),
                utf8_non_ascii,
            ))),
            tag("\""),
        )),
    )
    .parse(input)
}

fn string(input: Span) -> IResult<Span, Span> {
    context("string", alt((atom, quoted_string))).parse(input)
}

// ---------------------------------------------------------------------------
// MAIL FROM helpers
// ---------------------------------------------------------------------------

/// `dot-string = atom *("." atom)`
fn dot_string(input: Span) -> IResult<Span, Span> {
    context("dot-string", recognize((atom, many0(pair(tag("."), atom))))).parse(input)
}

/// `local-part = dot-string / quoted-string`
fn local_part(input: Span) -> IResult<Span, Span> {
    context("local-part", alt((dot_string, quoted_string))).parse(input)
}

/// `dcontent = %d33-90 / %d94-126`  — printable US-ASCII excluding `[`, `\`, `]`
fn dcontent(input: Span) -> IResult<Span, Span> {
    take_while1(|c: u8| (c >= 33 && c <= 90) || (c >= 94 && c <= 126)).parse(input)
}

/// The content inside an address literal `[...]`.
///
/// The IPv6 prefix is checked first so that `[IPv6:bad]` produces a
/// recoverable parse error rather than falling through to the general
/// address-literal branch (which would silently accept `IPv6` as a tag).
fn address_literal_content(input: Span) -> IResult<Span, Domain> {
    let is_ipv6 = input
        .fragment()
        .get(..5)
        .map(|b| b.eq_ignore_ascii_case(b"IPv6:"))
        .unwrap_or(false);

    if is_ipv6 {
        // Strictly parse as IPv6; no fallthrough to general literal on failure
        context(
            "ipv6-address-literal",
            map((tag_no_case("IPv6:"), ipv6_address), |(_, ip)| {
                Domain::V6(ip)
            }),
        )
        .parse(input)
    } else {
        alt((
            map(ipv4_address, Domain::V4),
            map_res(
                (
                    recognize(take_while1(|c: u8| c.is_ascii_alphanumeric() || c == b'-')),
                    tag(":"),
                    recognize(many1(dcontent)),
                ),
                |(tag_s, _colon, lit): (Span, _, Span)| -> Result<Domain, String> {
                    // Store the original "tag:literal" string as-is
                    let mut s = String::from_utf8(tag_s.fragment().to_vec())
                        .map_err(|_| "address_literal: invalid UTF-8 in tag".to_string())?;
                    s.push(':');
                    let lit_str = std::str::from_utf8(lit.fragment())
                        .map_err(|_| "address_literal: invalid UTF-8 in literal".to_string())?;
                    s.push_str(lit_str);
                    Ok(Domain::Tagged(s))
                },
            ),
        ))
        .parse(input)
    }
}

/// `address-literal = "[" ( IPv4 / "IPv6:" IPv6 / tag ":" dcontent ) "]"`
fn address_literal(input: Span) -> IResult<Span, Domain> {
    context(
        "address-literal",
        map((tag("["), address_literal_content, tag("]")), |(_, d, _)| d),
    )
    .parse(input)
}

/// `mailbox-domain = address-literal / domain-name`
fn mailbox_domain(input: Span) -> IResult<Span, Domain> {
    context(
        "mailbox-domain",
        alt((address_literal, map(domain_name, Domain::DomainName))),
    )
    .parse(input)
}

/// `mailbox = local-part "@" mailbox-domain`
fn mailbox(input: Span) -> IResult<Span, Mailbox> {
    context(
        "mailbox",
        map_res(
            (local_part, tag("@"), mailbox_domain),
            |(lp, _, dom): (Span, _, Domain)| -> Result<Mailbox, String> {
                // Convert the local_part bytes to a String
                // If the bytes are not valid UTF-8, return an error
                let local_part = String::from_utf8(lp.fragment().to_vec())
                    .map_err(|_| "invalid UTF-8 in local-part".to_string())?;
                Ok(Mailbox {
                    local_part,
                    domain: dom,
                })
            },
        ),
    )
    .parse(input)
}

/// `at-domain = "@" domain-name`  — returns the domain string (without the `@`)
fn at_domain(input: Span) -> IResult<Span, String> {
    map_res(
        (tag("@"), recognize(domain_name)),
        |(_, d): (Span, Span)| -> Result<String, String> {
            String::from_utf8(d.fragment().to_vec())
                .map_err(|_| "at_domain: invalid UTF-8 in domain".to_string())
        },
    )
    .parse(input)
}

/// `at-domain-list = at-domain *("," at-domain) ":"`
fn at_domain_list(input: Span) -> IResult<Span, Vec<String>> {
    context(
        "at-domain-list",
        map(
            (at_domain, many0((tag(","), at_domain)), tag(":")),
            |(first, rest, _)| {
                let mut v = vec![first];
                v.extend(rest.into_iter().map(|(_, d)| d));
                v
            },
        ),
    )
    .parse(input)
}

/// `null-sender = "<>"`
fn null_sender(input: Span) -> IResult<Span, ReversePath> {
    context("null-sender", map(tag("<>"), |_| ReversePath::NullSender)).parse(input)
}

/// `path = "<" [ at-domain-list ] mailbox ">"`
///
/// This is the core grammar element shared by both reverse-path and
/// forward-path. It parses the content between angle brackets and returns
/// a MailPath (optional source route + mailbox).
fn path(input: Span) -> IResult<Span, MailPath> {
    context(
        "path",
        map(
            (tag("<"), opt(at_domain_list), mailbox, tag(">")),
            |(_, domains, mb, _)| MailPath {
                at_domain_list: domains.unwrap_or_default(),
                mailbox: mb,
            },
        ),
    )
    .parse(input)
}

/// `reverse-path = null-sender / path / bare-mailbox`
///
/// Null sender is tried first so that `<>` is not consumed as the opening
/// `<` of a path.  Bare mailbox (no angle brackets) is accepted as a
/// leniency for non-conforming senders.
fn reverse_path(input: Span) -> IResult<Span, ReversePath> {
    context(
        "reverse-path",
        alt((
            null_sender,
            map(path, ReversePath::Path),
            map(mailbox, ReversePath::from),
        )),
    )
    .parse(input)
}

// ---------------------------------------------------------------------------
// RCPT TO helpers
// ---------------------------------------------------------------------------

/// `<Postmaster>` — the special no-domain postmaster address (RFC 5321 §4.1.1.3)
fn postmaster_path(input: Span) -> IResult<Span, ForwardPath> {
    context(
        "postmaster",
        map(tag_no_case("<Postmaster>"), |_| ForwardPath::Postmaster),
    )
    .parse(input)
}

/// `forward-path = "<Postmaster>" / "<" path-content ">" / bare-mailbox`
///
/// `<Postmaster>` is tried first so the literal string is not consumed as
/// the opening `<` of a regular path.  Bare mailbox (no angle brackets) is
/// accepted as a leniency for non-conforming senders.
fn forward_path(input: Span) -> IResult<Span, ForwardPath> {
    context(
        "forward-path",
        alt((
            postmaster_path,
            map(path, ForwardPath::Path),
            map(mailbox, ForwardPath::from),
        )),
    )
    .parse(input)
}

/// `esmtp-keyword = (ALPHA / DIGIT) *(ALPHA / DIGIT / "-")`
fn esmtp_keyword(input: Span) -> IResult<Span, Span> {
    context(
        "esmtp-keyword",
        recognize((
            take_while_m_n(1, 1, |c: u8| c.is_ascii_alphanumeric()),
            many0(take_while_m_n(1, 1, |c: u8| {
                c.is_ascii_alphanumeric() || c == b'-'
            })),
        )),
    )
    .parse(input)
}

/// `esmtp-value = 1*(%d33-60 / %d62-126 / UTF8-non-ASCII)`
///
/// RFC 5321 defines the base character range (printable ASCII excluding `=`,
/// SP, and controls).  RFC 6531 §3.3 extends this with `UTF8-non-ASCII` to
/// support internationalized ESMTP parameter values.
///
/// `many1(alt(ascii_run, utf8_non_ascii))` lets the `take_while1` arm greedily
/// consume consecutive ASCII bytes while `utf8_non_ascii` handles each
/// multi-byte codepoint.
fn esmtp_value(input: Span) -> IResult<Span, Span> {
    context(
        "esmtp-value",
        recognize(many1(alt((
            take_while1(|c: u8| (c >= 33 && c <= 60) || (c >= 62 && c <= 126)),
            utf8_non_ascii,
        )))),
    )
    .parse(input)
}

/// `esmtp-param = esmtp-keyword ["=" esmtp-value]`
fn esmtp_param(input: Span) -> IResult<Span, EsmtpParameter> {
    context(
        "esmtp-param",
        map_res(
            (esmtp_keyword, opt((tag("="), esmtp_value))),
            |(name, value): (Span, Option<(Span, Span)>)| -> Result<EsmtpParameter, String> {
                let name = String::from_utf8(name.fragment().to_vec())
                    .map_err(|_| "esmtp_param: invalid UTF-8 in name".to_string())?;
                let value = value
                    .map(|(_, v)| {
                        String::from_utf8(v.fragment().to_vec())
                            .map_err(|_| "esmtp_param: invalid UTF-8 in value".to_string())
                    })
                    .transpose()?;
                Ok(EsmtpParameter { name, value })
            },
        ),
    )
    .parse(input)
}

/// `mail-parameters = esmtp-param *(SP esmtp-param)`
fn mail_parameters(input: Span) -> IResult<Span, Vec<EsmtpParameter>> {
    context(
        "mail-parameters",
        map((esmtp_param, many0((wsp, esmtp_param))), |(first, rest)| {
            let mut params = vec![first];
            params.extend(rest.into_iter().map(|(_, p)| p));
            params
        }),
    )
    .parse(input)
}

// ---------------------------------------------------------------------------
// EHLO / HELO / LHLO parsers
// ---------------------------------------------------------------------------

/// `ehlo = "EHLO" SP ( Domain / address-literal ) CRLF`
///
/// Both HELO and LHLO reuse the same domain parser (permissive: address
/// literals are accepted for all three greeting commands).
/// Any failure in `mailbox_domain` (e.g. bad IP) falls through to Partial.
fn parse_ehlo(input: Span) -> IResult<Span, MaybePartialCommand> {
    context(
        "ehlo",
        alt((
            map(
                all_consuming((tag_no_case("EHLO"), wsp, mailbox_domain)),
                |(_, _, domain)| MaybePartialCommand::Full(Command::Ehlo(domain)),
            ),
            map(
                all_consuming((tag_no_case("EHLO"), wsp, anything)),
                |(_, _, remainder)| MaybePartialCommand::Partial {
                    verb: CommandVerb::Ehlo,
                    remainder: (*remainder).into(),
                },
            ),
            map(all_consuming(tag_no_case("EHLO")), |_| {
                MaybePartialCommand::Partial {
                    verb: CommandVerb::Ehlo,
                    remainder: BString::default(),
                }
            }),
        )),
    )
    .parse(input)
}

/// `helo = "HELO" SP Domain CRLF`
///
/// Permissive: also accepts address literals (matches parser.rs behaviour).
fn parse_helo(input: Span) -> IResult<Span, MaybePartialCommand> {
    context(
        "helo",
        alt((
            map(
                all_consuming((tag_no_case("HELO"), wsp, mailbox_domain)),
                |(_, _, domain)| MaybePartialCommand::Full(Command::Helo(domain)),
            ),
            map(
                all_consuming((tag_no_case("HELO"), wsp, anything)),
                |(_, _, remainder)| MaybePartialCommand::Partial {
                    verb: CommandVerb::Helo,
                    remainder: (*remainder).into(),
                },
            ),
            map(all_consuming(tag_no_case("HELO")), |_| {
                MaybePartialCommand::Partial {
                    verb: CommandVerb::Helo,
                    remainder: BString::default(),
                }
            }),
        )),
    )
    .parse(input)
}

/// `lhlo = "LHLO" SP ( Domain / address-literal ) CRLF`  (RFC 2033 LMTP)
fn parse_lhlo(input: Span) -> IResult<Span, MaybePartialCommand> {
    context(
        "lhlo",
        alt((
            map(
                all_consuming((tag_no_case("LHLO"), wsp, mailbox_domain)),
                |(_, _, domain)| MaybePartialCommand::Full(Command::Lhlo(domain)),
            ),
            map(
                all_consuming((tag_no_case("LHLO"), wsp, anything)),
                |(_, _, remainder)| MaybePartialCommand::Partial {
                    verb: CommandVerb::Lhlo,
                    remainder: (*remainder).into(),
                },
            ),
            map(all_consuming(tag_no_case("LHLO")), |_| {
                MaybePartialCommand::Partial {
                    verb: CommandVerb::Lhlo,
                    remainder: BString::default(),
                }
            }),
        )),
    )
    .parse(input)
}

// ---------------------------------------------------------------------------
// AUTH parser
// ---------------------------------------------------------------------------

/// `sasl-mechanism = 1*(ALPHA / DIGIT / "-")`
fn sasl_mechanism(input: Span) -> IResult<Span, String> {
    context(
        "sasl-mechanism",
        map(
            take_while1(|c: u8| c.is_ascii_alphanumeric() || c == b'-'),
            |s: Span| {
                // sasl_mechanism only accepts ASCII bytes, so UTF-8 conversion always succeeds
                String::from_utf8(s.fragment().to_vec()).expect("sasl_mechanism guaranteed ASCII")
            },
        ),
    )
    .parse(input)
}

/// `auth-initial-response = base64 / "="`
///
/// Matches base64 characters `[A-Za-z0-9+/=]+`.  The single `"="` (empty
/// initial response) is a subset of this pattern, so no special case is
/// needed.
fn auth_initial_response(input: Span) -> IResult<Span, String> {
    context(
        "auth-initial-response",
        map(
            take_while1(|c: u8| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'='),
            |s: Span| {
                // auth_initial_response only accepts ASCII bytes, so UTF-8 conversion always succeeds
                String::from_utf8(s.fragment().to_vec())
                    .expect("auth_initial_response guaranteed ASCII")
            },
        ),
    )
    .parse(input)
}

/// `auth = "AUTH" SP mechanism [SP initial-response]`
fn parse_auth(input: Span) -> IResult<Span, MaybePartialCommand> {
    context(
        "auth",
        alt((
            // Arm 1: complete successful parse
            map(
                all_consuming((
                    tag_no_case("AUTH"),
                    wsp,
                    sasl_mechanism,
                    opt((wsp, auth_initial_response)),
                )),
                |(_, _, sasl_mech, resp)| match resp {
                    Some((_, r)) => MaybePartialCommand::Full(Command::Auth {
                        sasl_mech,
                        initial_response: Some(r),
                    }),
                    None => MaybePartialCommand::Full(Command::Auth {
                        sasl_mech,
                        initial_response: None,
                    }),
                },
            ),
            // Arm 2: "AUTH" + whitespace + anything → Partial
            map(
                all_consuming((tag_no_case("AUTH"), wsp, anything)),
                |(_, _, remainder)| MaybePartialCommand::Partial {
                    verb: CommandVerb::Auth,
                    remainder: (*remainder).into(),
                },
            ),
            // Arm 3: "AUTH" alone → Partial
            map(all_consuming(tag_no_case("AUTH")), |_| {
                MaybePartialCommand::Partial {
                    verb: CommandVerb::Auth,
                    remainder: BString::default(),
                }
            }),
        )),
    )
    .parse(input)
}

// ---------------------------------------------------------------------------
// XCLIENT parser
// ---------------------------------------------------------------------------

/// Decode an xtext-encoded byte slice (RFC 3461 §4).
///
/// xtext characters are printable ASCII in `\x21`–`\x7e` where `+XX`
/// introduces a hex-encoded byte.  Returns an error on a truncated or
/// invalid hex escape.
fn xtext_decode(encoded: &[u8]) -> Result<String, String> {
    let mut result: Vec<u8> = Vec::with_capacity(encoded.len());
    let mut i = 0;
    while i < encoded.len() {
        if encoded[i] == b'+' {
            if i + 2 >= encoded.len() {
                return Err(format!("xtext_decode: truncated hex escape at byte {i}"));
            }
            let hi = hex_nibble(encoded[i + 1]).map_err(|e| format!("xtext_decode: {e}"))?;
            let lo = hex_nibble(encoded[i + 2]).map_err(|e| format!("xtext_decode: {e}"))?;
            result.push((hi << 4) | lo);
            i += 3;
        } else {
            result.push(encoded[i]);
            i += 1;
        }
    }
    String::from_utf8(result)
        .map_err(|_| "xtext_decode: invalid UTF-8 in decoded value".to_string())
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("invalid hex digit '{}'", b as char)),
    }
}

/// Raw xtext value: printable non-space ASCII (`\x21`–`\x7e`), which
/// includes the `+` that introduces a hex escape.  Space terminates the
/// value in the XCLIENT parameter list.
fn xclient_xtext_value(input: Span) -> IResult<Span, Span> {
    context(
        "xclient-xtext-value",
        take_while1(|c: u8| c >= 33 && c <= 126),
    )
    .parse(input)
}

/// One XCLIENT parameter: `name "=" xtext-value`
///
/// The value is xtext-decoded via `map_res`; a malformed escape sequence
/// produces a recoverable nom error so `alt` can fall through to Partial.
fn xclient_param(input: Span) -> IResult<Span, XClientParameter> {
    context(
        "xclient-param",
        map_res(
            (esmtp_keyword, tag("="), xclient_xtext_value),
            |(name, _, value): (Span, _, Span)| -> Result<XClientParameter, String> {
                let name = String::from_utf8(name.fragment().to_vec())
                    .map_err(|_| "xclient_param: invalid UTF-8 in name".to_string())?;
                let value = xtext_decode(value.fragment())?;
                Ok(XClientParameter { name, value })
            },
        ),
    )
    .parse(input)
}

/// `xclient-params = xclient-param *(SP xclient-param)`
fn xclient_params(input: Span) -> IResult<Span, Vec<XClientParameter>> {
    context(
        "xclient-params",
        map(
            (xclient_param, many0((wsp, xclient_param))),
            |(first, rest)| {
                let mut params = vec![first];
                params.extend(rest.into_iter().map(|(_, p)| p));
                params
            },
        ),
    )
    .parse(input)
}

/// `xclient = "XCLIENT" SP xclient-params`
fn parse_xclient(input: Span) -> IResult<Span, MaybePartialCommand> {
    context(
        "xclient",
        alt((
            // Arm 1: complete successful parse
            map(
                all_consuming((tag_no_case("XCLIENT"), wsp, xclient_params)),
                |(_, _, params)| MaybePartialCommand::Full(Command::XClient(params)),
            ),
            // Arm 2: "XCLIENT" + whitespace + anything → Partial
            map(
                all_consuming((tag_no_case("XCLIENT"), wsp, anything)),
                |(_, _, remainder)| MaybePartialCommand::Partial {
                    verb: CommandVerb::XClient,
                    remainder: (*remainder).into(),
                },
            ),
            // Arm 3: "XCLIENT" alone → Partial
            map(all_consuming(tag_no_case("XCLIENT")), |_| {
                MaybePartialCommand::Partial {
                    verb: CommandVerb::XClient,
                    remainder: BString::default(),
                }
            }),
        )),
    )
    .parse(input)
}

/// `mail = "MAIL FROM:" reverse-path [SP mail-parameters]`
///
/// Returns `Full(Command::MailFrom { … })` on a complete successful parse.
///
/// Any failure after the `MAIL` keyword — including an invalid IPv4/IPv6
/// address or domain name in the sender address — falls through to a
/// `Partial { verb: CommandVerb::Mail, … }` result instead of a hard error.
/// No `cut` is used anywhere in the full-parse arm so that all sub-parser
/// failures remain recoverable and `alt` can try the fallback arms.
fn parse_mail_from(input: Span) -> IResult<Span, MaybePartialCommand> {
    context(
        "mail-from",
        alt((
            // Arm 1: complete successful parse
            map(
                all_consuming((
                    tag_no_case("MAIL"),
                    wsp,
                    tag_no_case("FROM:"),
                    reverse_path,
                    opt(map((wsp, mail_parameters), |(_, p)| p)),
                )),
                |(_, _, _, address, parameters)| {
                    MaybePartialCommand::Full(Command::MailFrom {
                        address,
                        parameters: parameters.unwrap_or_default(),
                    })
                },
            ),
            // Arm 2: "MAIL" + whitespace + anything → Partial
            // This catches bad addresses (invalid IP, domain, syntax)
            map(
                all_consuming((tag_no_case("MAIL"), wsp, anything)),
                |(_, _, remainder)| MaybePartialCommand::Partial {
                    verb: CommandVerb::Mail,
                    remainder: (*remainder).into(),
                },
            ),
            // Arm 3: "MAIL" alone → Partial (incomplete command)
            map(all_consuming(tag_no_case("MAIL")), |_| {
                MaybePartialCommand::Partial {
                    verb: CommandVerb::Mail,
                    remainder: BString::default(),
                }
            }),
        )),
    )
    .parse(input)
}

/// `rcpt = "RCPT TO:" forward-path [SP mail-parameters]`
///
/// Returns `Full(Command::RcptTo { … })` on a complete successful parse.
///
/// Any failure after the `RCPT` keyword — including an invalid IPv4/IPv6
/// address or domain name in the recipient address — falls through to a
/// `Partial { verb: CommandVerb::Rcpt, … }` result instead of a hard error.
/// No `cut` is used anywhere in the full-parse arm so that all sub-parser
/// failures remain recoverable and `alt` can try the fallback arms.
fn parse_rcpt_to(input: Span) -> IResult<Span, MaybePartialCommand> {
    context(
        "rcpt-to",
        alt((
            // Arm 1: complete successful parse
            map(
                all_consuming((
                    tag_no_case("RCPT"),
                    wsp,
                    tag_no_case("TO:"),
                    forward_path,
                    opt(map((wsp, mail_parameters), |(_, p)| p)),
                )),
                |(_, _, _, address, parameters)| {
                    MaybePartialCommand::Full(Command::RcptTo {
                        address,
                        parameters: parameters.unwrap_or_default(),
                    })
                },
            ),
            // Arm 2: "RCPT" + whitespace + anything → Partial
            // This catches bad addresses (invalid IP, domain, syntax)
            map(
                all_consuming((tag_no_case("RCPT"), wsp, anything)),
                |(_, _, remainder)| MaybePartialCommand::Partial {
                    verb: CommandVerb::Rcpt,
                    remainder: (*remainder).into(),
                },
            ),
            // Arm 3: "RCPT" alone → Partial (incomplete command)
            map(all_consuming(tag_no_case("RCPT")), |_| {
                MaybePartialCommand::Partial {
                    verb: CommandVerb::Rcpt,
                    remainder: BString::default(),
                }
            }),
        )),
    )
    .parse(input)
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

/// Return the lowercase hex digit character for a nibble value (0–15).
fn hex_nibble_lower(n: u8) -> u8 {
    if n < 10 {
        b'0' + n
    } else {
        b'a' + n - 10
    }
}

/// Xtext-encode a byte slice (RFC 3461 §4).
///
/// Bytes in the xchar range (`\x21`–`\x7e` except `+` and `=`) are copied
/// unchanged.  All other byte values are encoded as `+XX` where `XX` is two
/// lowercase hex digits.
fn xtext_encode_bytes(value: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(value.len());
    for &b in value {
        if b >= 33 && b <= 126 && b != b'+' && b != b'=' {
            result.push(b);
        } else {
            result.push(b'+');
            result.push(hex_nibble_lower(b >> 4));
            result.push(hex_nibble_lower(b & 0x0f));
        }
    }
    result
}

/// Encode a `Domain` as the ASCII text that appears in a command line.
///
/// - `DomainName` → ASCII/punycode-normalised domain string
/// - `V4` → `[{ip}]`
/// - `V6` → `[IPv6:{ip}]`
/// - `Tagged` → `[{tag:literal}]`
fn encode_domain(domain: &Domain) -> BString {
    match domain {
        Domain::DomainName(s) => BString::from(s.to_string()),
        Domain::V4(ip) => BString::from(format!("[{}]", ip)),
        Domain::V6(ip) => BString::from(format!("[IPv6:{}]", ip)),
        Domain::Tagged(s) => BString::from(format!("[{}]", s)),
    }
}

/// Encode a `MailPath` as `local-part "@" domain` bytes.
///
/// The `at_domain_list` (source route) is intentionally **not** re-encoded:
/// RFC 5321 §4.1.2 says source routes MUST be accepted, SHOULD NOT be
/// generated, and SHOULD be ignored.
fn encode_mail_path(path: &MailPath) -> Vec<u8> {
    let mut buf = path.mailbox.local_part.as_bytes().to_vec();
    buf.push(b'@');
    buf.extend_from_slice(encode_domain(&path.mailbox.domain).as_ref());
    buf
}

/// Encode the content that goes **between** the angle brackets of
/// `MAIL FROM:<…>`.
///
/// `NullSender` → empty (produces `MAIL FROM:<>`).
/// `Path` → `encode_mail_path` result.
fn encode_reverse_path(rp: &ReversePath) -> Vec<u8> {
    match rp {
        ReversePath::NullSender => vec![],
        ReversePath::Path(path) => encode_mail_path(path),
    }
}

/// Encode the content that goes **between** the angle brackets of
/// `RCPT TO:<…>`.
///
/// `Postmaster` → `b"Postmaster"`.
/// `Path` → `encode_mail_path` result.
fn encode_forward_path(fp: &ForwardPath) -> Vec<u8> {
    match fp {
        ForwardPath::Postmaster => b"Postmaster".to_vec(),
        ForwardPath::Path(path) => encode_mail_path(path),
    }
}

/// Encode a slice of `EsmtpParameter` as the optional suffix of a
/// `MAIL FROM` or `RCPT TO` command: `*(SP keyword ["=" value])`.
///
/// Returns an empty `Vec` when `params` is empty.
fn encode_esmtp_params(params: &[EsmtpParameter]) -> Vec<u8> {
    let mut buf = Vec::new();
    for p in params {
        buf.push(b' ');
        buf.extend_from_slice(p.name.as_bytes());
        if let Some(v) = &p.value {
            buf.push(b'=');
            buf.extend_from_slice(v.as_bytes());
        }
    }
    buf
}

/// Encode a slice of `XClientParameter` as `*(SP name "=" xtext-value)`.
///
/// Each parameter value is xtext-encoded before writing.
fn encode_xclient_params(params: &[XClientParameter]) -> Vec<u8> {
    let mut buf = Vec::new();
    for p in params {
        buf.push(b' ');
        buf.extend_from_slice(p.name.as_bytes());
        buf.push(b'=');
        buf.extend(xtext_encode_bytes(p.value.as_bytes()));
    }
    buf
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_string() {
        k9::snapshot!(
            BStr::new(&parse_with("hello".as_bytes(), string).unwrap()),
            "hello"
        );
        k9::snapshot!(
            BStr::new(&parse_with("\"hello\"".as_bytes(), string).unwrap()),
            "\"hello\""
        );
        k9::snapshot!(
            BStr::new(&parse_with("\"hello world\"".as_bytes(), string).unwrap()),
            "\"hello world\""
        );
        k9::snapshot!(
            parse_with("hello world".as_bytes(), string),
            r#"
Err(
    "Error at line 1, in Eof:
hello world
     ^_____

",
)
"#
        );
    }

    #[test]
    fn test_bogus() {
        k9::snapshot!(
            Command::parse("bogus"),
            r#"
Ok(
    Full(Command("bogus\r
")),
)
"#
        );
    }

    // ------------------------------------------------------------------
    // EHLO tests
    // ------------------------------------------------------------------

    #[test]
    fn test_ehlo_domain_name() {
        k9::assert_equal!(
            unwrapper(Command::parse("EHLO example.com")),
            MaybePartialCommand::Full(Command::Ehlo(Domain::DomainName(
                "example.com".parse().unwrap()
            )))
        );
    }

    #[test]
    fn test_ehlo_case_insensitive() {
        k9::assert_equal!(
            unwrapper(Command::parse("ehlo example.com")),
            MaybePartialCommand::Full(Command::Ehlo(Domain::DomainName(
                "example.com".parse().unwrap()
            )))
        );
    }

    #[test]
    fn test_ehlo_ipv4_literal() {
        k9::assert_equal!(
            unwrapper(Command::parse("EHLO [10.0.0.1]")),
            MaybePartialCommand::Full(Command::Ehlo(Domain::V4("10.0.0.1".parse().unwrap())))
        );
    }

    #[test]
    fn test_ehlo_ipv6_literal() {
        k9::assert_equal!(
            unwrapper(Command::parse("EHLO [IPv6:::1]")),
            MaybePartialCommand::Full(Command::Ehlo(Domain::V6("::1".parse().unwrap())))
        );
    }

    #[test]
    fn test_ehlo_tagged_literal() {
        k9::assert_equal!(
            unwrapper(Command::parse("EHLO [future:something]")),
            MaybePartialCommand::Full(Command::Ehlo(Domain::Tagged("future:something".into(),)))
        );
    }

    #[test]
    fn test_ehlo_invalid_ipv4_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("EHLO [999.999.999.999]")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Ehlo,
                remainder: "[999.999.999.999]".into(),
            }
        );
    }

    #[test]
    fn test_ehlo_invalid_ipv6_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("EHLO [IPv6:not-an-ipv6]")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Ehlo,
                remainder: "[IPv6:not-an-ipv6]".into(),
            }
        );
    }

    #[test]
    fn test_ehlo_alone_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("EHLO")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Ehlo,
                remainder: "".into(),
            }
        );
    }

    #[test]
    fn test_ehlo_with_garbage_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("EHLO !!invalid!!")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Ehlo,
                remainder: "!!invalid!!".into(),
            }
        );
    }

    // ------------------------------------------------------------------
    // HELO tests
    // ------------------------------------------------------------------

    #[test]
    fn test_helo_domain_name() {
        k9::assert_equal!(
            unwrapper(Command::parse("HELO example.com")),
            MaybePartialCommand::Full(Command::Helo(Domain::DomainName(
                "example.com".parse().unwrap()
            )))
        );
    }

    #[test]
    fn test_helo_case_insensitive() {
        k9::assert_equal!(
            unwrapper(Command::parse("helo example.com")),
            MaybePartialCommand::Full(Command::Helo(Domain::DomainName(
                "example.com".parse().unwrap()
            )))
        );
    }

    #[test]
    fn test_helo_ipv4_literal() {
        // Permissive: address literals accepted for HELO
        k9::assert_equal!(
            unwrapper(Command::parse("HELO [10.0.0.1]")),
            MaybePartialCommand::Full(Command::Helo(Domain::V4("10.0.0.1".parse().unwrap())))
        );
    }

    #[test]
    fn test_helo_alone_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("HELO")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Helo,
                remainder: "".into(),
            }
        );
    }

    #[test]
    fn test_helo_with_garbage_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("HELO !!invalid!!")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Helo,
                remainder: "!!invalid!!".into(),
            }
        );
    }

    // ------------------------------------------------------------------
    // LHLO tests
    // ------------------------------------------------------------------

    #[test]
    fn test_lhlo_domain_name() {
        k9::assert_equal!(
            unwrapper(Command::parse("LHLO example.com")),
            MaybePartialCommand::Full(Command::Lhlo(Domain::DomainName(
                "example.com".parse().unwrap()
            )))
        );
    }

    #[test]
    fn test_lhlo_case_insensitive() {
        k9::assert_equal!(
            unwrapper(Command::parse("lhlo example.com")),
            MaybePartialCommand::Full(Command::Lhlo(Domain::DomainName(
                "example.com".parse().unwrap()
            )))
        );
    }

    #[test]
    fn test_lhlo_ipv4_literal() {
        k9::assert_equal!(
            unwrapper(Command::parse("LHLO [10.0.0.1]")),
            MaybePartialCommand::Full(Command::Lhlo(Domain::V4("10.0.0.1".parse().unwrap())))
        );
    }

    #[test]
    fn test_lhlo_ipv6_literal() {
        k9::assert_equal!(
            unwrapper(Command::parse("LHLO [IPv6:::1]")),
            MaybePartialCommand::Full(Command::Lhlo(Domain::V6("::1".parse().unwrap())))
        );
    }

    #[test]
    fn test_lhlo_invalid_ipv4_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("LHLO [999.999.999.999]")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Lhlo,
                remainder: "[999.999.999.999]".into(),
            }
        );
    }

    #[test]
    fn test_lhlo_alone_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("LHLO")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Lhlo,
                remainder: "".into(),
            }
        );
    }

    // ------------------------------------------------------------------
    // AUTH tests
    // ------------------------------------------------------------------

    #[test]
    fn test_auth_mechanism_only() {
        k9::assert_equal!(
            unwrapper(Command::parse("AUTH PLAIN")),
            MaybePartialCommand::Full(Command::Auth {
                sasl_mech: "PLAIN".into(),
                initial_response: None,
            })
        );
    }

    #[test]
    fn test_auth_with_initial_response() {
        k9::assert_equal!(
            unwrapper(Command::parse("AUTH PLAIN dXNlcjpwYXNz")),
            MaybePartialCommand::Full(Command::Auth {
                sasl_mech: "PLAIN".into(),
                initial_response: Some("dXNlcjpwYXNz".into()),
            })
        );
    }

    #[test]
    fn test_auth_empty_initial_response() {
        // "=" signals an empty initial response (RFC 4954)
        k9::assert_equal!(
            unwrapper(Command::parse("AUTH PLAIN =")),
            MaybePartialCommand::Full(Command::Auth {
                sasl_mech: "PLAIN".into(),
                initial_response: Some("=".into()),
            })
        );
    }

    #[test]
    fn test_auth_hyphenated_mechanism() {
        k9::assert_equal!(
            unwrapper(Command::parse("AUTH CRAM-MD5")),
            MaybePartialCommand::Full(Command::Auth {
                sasl_mech: "CRAM-MD5".into(),
                initial_response: None,
            })
        );
    }

    #[test]
    fn test_auth_case_insensitive() {
        k9::assert_equal!(
            unwrapper(Command::parse("auth plain")),
            MaybePartialCommand::Full(Command::Auth {
                sasl_mech: "plain".into(),
                initial_response: None,
            })
        );
    }

    #[test]
    fn test_auth_alone_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("AUTH")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Auth,
                remainder: "".into(),
            }
        );
    }

    #[test]
    fn test_auth_with_garbage_is_partial() {
        // Mechanism contains invalid characters → Partial
        k9::assert_equal!(
            unwrapper(Command::parse("AUTH !!bad!!")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Auth,
                remainder: "!!bad!!".into(),
            }
        );
    }

    // ------------------------------------------------------------------
    // XCLIENT tests
    // ------------------------------------------------------------------

    #[test]
    fn test_xclient_single_param() {
        k9::assert_equal!(
            unwrapper(Command::parse("XCLIENT NAME=foo.example.com")),
            MaybePartialCommand::Full(Command::XClient(vec![XClientParameter {
                name: "NAME".into(),
                value: "foo.example.com".into(),
            }]))
        );
    }

    #[test]
    fn test_xclient_multiple_params() {
        k9::assert_equal!(
            unwrapper(Command::parse("XCLIENT NAME=foo.example.com ADDR=10.0.0.1")),
            MaybePartialCommand::Full(Command::XClient(vec![
                XClientParameter {
                    name: "NAME".into(),
                    value: "foo.example.com".into(),
                },
                XClientParameter {
                    name: "ADDR".into(),
                    value: "10.0.0.1".into(),
                },
            ]))
        );
    }

    #[test]
    fn test_xclient_xtext_hex_escape() {
        // '+40' decodes to '@'
        k9::assert_equal!(
            unwrapper(Command::parse("XCLIENT NAME=user+40example.com")),
            MaybePartialCommand::Full(Command::XClient(vec![XClientParameter {
                name: "NAME".into(),
                value: "user@example.com".into(),
            }]))
        );
    }

    #[test]
    fn test_xclient_case_insensitive() {
        k9::assert_equal!(
            unwrapper(Command::parse("xclient NAME=host")),
            MaybePartialCommand::Full(Command::XClient(vec![XClientParameter {
                name: "NAME".into(),
                value: "host".into(),
            }]))
        );
    }

    #[test]
    fn test_xclient_alone_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("XCLIENT")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::XClient,
                remainder: "".into(),
            }
        );
    }

    #[test]
    fn test_xclient_invalid_xtext_is_partial() {
        // '+' not followed by two hex digits → xtext_decode fails → Partial
        k9::assert_equal!(
            unwrapper(Command::parse("XCLIENT NAME=bad+ZZ")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::XClient,
                remainder: "NAME=bad+ZZ".into(),
            }
        );
    }

    #[test]
    fn test_xclient_garbage_is_partial() {
        // No '=' separator → xclient_param fails → Partial
        k9::assert_equal!(
            unwrapper(Command::parse("XCLIENT noequals")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::XClient,
                remainder: "noequals".into(),
            }
        );
    }

    #[test]
    fn test_xclient_parameter_methods() {
        // Parse an XCLIENT command with an IP address
        let cmd = unwrapper(Command::parse("XCLIENT ADDR=192.168.1.1"));
        let params = match cmd {
            MaybePartialCommand::Full(Command::XClient(params)) => params,
            _ => panic!("Expected XCLIENT command"),
        };

        // Test is_name method
        k9::assert_equal!(params[0].is_name("ADDR"), true);
        k9::assert_equal!(params[0].is_name("addr"), true);
        k9::assert_equal!(params[0].is_name("NAME"), false);

        // Test parse method with IpAddr
        let ip: std::net::IpAddr = params[0].parse().expect("Failed to parse IP address");
        k9::assert_equal!(
            ip,
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 1))
        );
    }

    #[test]
    fn test_xclient_parameter_parse_invalid_ip() {
        // Test parsing with an invalid IP address string
        let cmd = unwrapper(Command::parse("XCLIENT ADDR=not-an-ip"));
        let params = match cmd {
            MaybePartialCommand::Full(Command::XClient(params)) => params,
            _ => panic!("Expected XCLIENT command"),
        };

        let result: Result<std::net::IpAddr, _> = params[0].parse();
        k9::assert_equal!(result.unwrap_err(), "invalid IP address syntax");
    }

    // ------------------------------------------------------------------
    // MAIL FROM tests
    // ------------------------------------------------------------------

    fn mail_path(local: &str, domain: Domain) -> ReversePath {
        Mailbox {
            local_part: local.into(),
            domain,
        }
        .into()
    }

    #[test]
    fn test_mail_from_domain_name() {
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:<user@host>")),
            MaybePartialCommand::Full(Command::MailFrom {
                address: mail_path("user", Domain::DomainName("host".parse().unwrap())),
                parameters: vec![],
            })
        );
        // Case-insensitive verb and keyword
        k9::assert_equal!(
            unwrapper(Command::parse("mail from:<user@host>")),
            MaybePartialCommand::Full(Command::MailFrom {
                address: mail_path("user", Domain::DomainName("host".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_mail_from_null_sender() {
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:<>")),
            MaybePartialCommand::Full(Command::MailFrom {
                address: ReversePath::NullSender,
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_mail_from_ipv4() {
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:<user@[10.0.0.1]>")),
            MaybePartialCommand::Full(Command::MailFrom {
                address: mail_path("user", Domain::V4("10.0.0.1".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_mail_from_ipv6() {
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:<user@[IPv6:::1]>")),
            MaybePartialCommand::Full(Command::MailFrom {
                address: mail_path("user", Domain::V6("::1".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_mail_from_tagged_literal() {
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:<user@[future:something]>")),
            MaybePartialCommand::Full(Command::MailFrom {
                address: mail_path("user", Domain::Tagged("future:something".into())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_mail_from_esmtp_params() {
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:<user@host> foo bar=baz")),
            MaybePartialCommand::Full(Command::MailFrom {
                address: mail_path("user", Domain::DomainName("host".parse().unwrap())),
                parameters: vec![
                    EsmtpParameter {
                        name: "foo".into(),
                        value: None,
                    },
                    EsmtpParameter {
                        name: "bar".into(),
                        value: Some("baz".into()),
                    },
                ],
            })
        );
    }

    #[test]
    fn test_mail_from_bare_address() {
        // No angle brackets — accepted as a leniency
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:user@host")),
            MaybePartialCommand::Full(Command::MailFrom {
                address: mail_path("user", Domain::DomainName("host".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_mail_from_at_domain_list() {
        k9::assert_equal!(
            unwrapper(Command::parse(
                "MAIL FROM:<@hosta.int,@jkl.org:userc@d.bar.org>"
            )),
            MaybePartialCommand::Full(Command::MailFrom {
                address: ReversePath::Path(MailPath {
                    at_domain_list: vec!["hosta.int".into(), "jkl.org".into()],
                    mailbox: Mailbox {
                        local_part: "userc".into(),
                        domain: Domain::DomainName("d.bar.org".parse().unwrap()),
                    },
                }),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_mail_from_invalid_ipv4_is_partial() {
        // Bad IPv4 inside brackets → Partial, not a hard error
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:<user@[999.999.999.999]>")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Mail,
                remainder: "FROM:<user@[999.999.999.999]>".into(),
            }
        );
    }

    #[test]
    fn test_mail_from_invalid_ipv6_is_partial() {
        // Bad IPv6 after "IPv6:" prefix → Partial, not a hard error
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL FROM:<user@[IPv6:not-an-ipv6]>")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Mail,
                remainder: "FROM:<user@[IPv6:not-an-ipv6]>".into(),
            }
        );
    }

    #[test]
    fn test_mail_alone_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Mail,
                remainder: "".into(),
            }
        );
    }

    #[test]
    fn test_mail_with_garbage_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("MAIL garbage")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Mail,
                remainder: "garbage".into(),
            }
        );
    }

    // ------------------------------------------------------------------
    // RCPT TO tests
    // ------------------------------------------------------------------

    fn rcpt_path(local: &str, domain: Domain) -> ForwardPath {
        Mailbox {
            local_part: local.into(),
            domain,
        }
        .into()
    }

    #[test]
    fn test_rcpt_to_domain_name() {
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<user@host>")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: rcpt_path("user", Domain::DomainName("host".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_case_insensitive() {
        // Verb and keyword are case-insensitive
        k9::assert_equal!(
            unwrapper(Command::parse("rcpt to:<user@host>")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: rcpt_path("user", Domain::DomainName("host".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_postmaster() {
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<Postmaster>")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: ForwardPath::Postmaster,
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_postmaster_lowercase() {
        // <Postmaster> is case-insensitive per RFC 5321
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<postmaster>")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: ForwardPath::Postmaster,
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_ipv4() {
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<user@[10.0.0.1]>")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: rcpt_path("user", Domain::V4("10.0.0.1".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_ipv6() {
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<user@[IPv6:::1]>")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: rcpt_path("user", Domain::V6("::1".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_tagged_literal() {
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<user@[future:something]>")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: rcpt_path("user", Domain::Tagged("future:something".into())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_esmtp_params() {
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<user@host> foo bar=baz")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: rcpt_path("user", Domain::DomainName("host".parse().unwrap())),
                parameters: vec![
                    EsmtpParameter {
                        name: "foo".into(),
                        value: None,
                    },
                    EsmtpParameter {
                        name: "bar".into(),
                        value: Some("baz".into()),
                    },
                ],
            })
        );
    }

    #[test]
    fn test_rcpt_to_bare_address() {
        // No angle brackets — accepted as a leniency
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:user@host")),
            MaybePartialCommand::Full(Command::RcptTo {
                address: rcpt_path("user", Domain::DomainName("host".parse().unwrap())),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_at_domain_list() {
        k9::assert_equal!(
            unwrapper(Command::parse(
                "RCPT TO:<@hosta.int,@jkl.org:userc@d.bar.org>"
            )),
            MaybePartialCommand::Full(Command::RcptTo {
                address: ForwardPath::Path(MailPath {
                    at_domain_list: vec!["hosta.int".into(), "jkl.org".into()],
                    mailbox: Mailbox {
                        local_part: "userc".into(),
                        domain: Domain::DomainName("d.bar.org".parse().unwrap()),
                    },
                }),
                parameters: vec![],
            })
        );
    }

    #[test]
    fn test_rcpt_to_invalid_ipv4_is_partial() {
        // Bad IPv4 inside brackets → Partial, not a hard error
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<user@[999.999.999.999]>")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Rcpt,
                remainder: "TO:<user@[999.999.999.999]>".into(),
            }
        );
    }

    #[test]
    fn test_rcpt_to_invalid_ipv6_is_partial() {
        // Bad IPv6 after "IPv6:" prefix → Partial, not a hard error
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT TO:<user@[IPv6:not-an-ipv6]>")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Rcpt,
                remainder: "TO:<user@[IPv6:not-an-ipv6]>".into(),
            }
        );
    }

    #[test]
    fn test_rcpt_alone_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Rcpt,
                remainder: "".into(),
            }
        );
    }

    #[test]
    fn test_rcpt_with_garbage_is_partial() {
        k9::assert_equal!(
            unwrapper(Command::parse("RCPT garbage")),
            MaybePartialCommand::Partial {
                verb: CommandVerb::Rcpt,
                remainder: "garbage".into(),
            }
        );
    }

    // ------------------------------------------------------------------
    // encode / encode_str tests
    // ------------------------------------------------------------------

    /// Parse a command from a string, encode it, and assert the encoded
    /// output equals `expected`.  Then parse the encoded output and assert
    /// the result equals the original parsed command (round-trip).
    fn assert_encode(input: &str, expected: &str) {
        let cmd = match unwrapper(Command::parse(input)) {
            MaybePartialCommand::Full(c) => c,
            other => panic!("expected Full, got {other:?}"),
        };
        let encoded = cmd.encode();
        k9::assert_equal!(encoded, BString::from(expected));
        // Round-trip: parsing the encoded form must reproduce the command
        k9::assert_equal!(
            unwrapper(Command::parse(encoded.clone())),
            MaybePartialCommand::Full(cmd)
        );
    }

    #[test]
    fn test_encode_ehlo_domain() {
        assert_encode("EHLO example.com", "EHLO example.com\r\n");
    }

    #[test]
    fn test_encode_ehlo_ipv4() {
        assert_encode("EHLO [10.0.0.1]", "EHLO [10.0.0.1]\r\n");
    }

    #[test]
    fn test_encode_ehlo_ipv6() {
        assert_encode("EHLO [IPv6:::1]", "EHLO [IPv6:::1]\r\n");
    }

    #[test]
    fn test_encode_ehlo_tagged() {
        assert_encode("EHLO [future:something]", "EHLO [future:something]\r\n");
    }

    #[test]
    fn test_encode_helo() {
        assert_encode("HELO mail.example.com", "HELO mail.example.com\r\n");
    }

    #[test]
    fn test_encode_lhlo() {
        assert_encode("LHLO mail.example.com", "LHLO mail.example.com\r\n");
    }

    #[test]
    fn test_encode_noop_none() {
        assert_encode("NOOP", "NOOP\r\n");
    }

    #[test]
    fn test_encode_noop_some() {
        assert_encode("NOOP something", "NOOP something\r\n");
    }

    #[test]
    fn test_encode_help_none() {
        assert_encode("HELP", "HELP\r\n");
    }

    #[test]
    fn test_encode_help_some() {
        assert_encode("HELP MAIL", "HELP MAIL\r\n");
    }

    #[test]
    fn test_encode_vrfy_none() {
        assert_encode("VRFY", "VRFY\r\n");
    }

    #[test]
    fn test_encode_vrfy_some() {
        assert_encode("VRFY user", "VRFY user\r\n");
    }

    #[test]
    fn test_encode_expn_none() {
        assert_encode("EXPN", "EXPN\r\n");
    }

    #[test]
    fn test_encode_expn_some() {
        assert_encode("EXPN list", "EXPN list\r\n");
    }

    #[test]
    fn test_encode_data() {
        assert_encode("DATA", "DATA\r\n");
    }

    #[test]
    fn test_encode_data_dot() {
        // DataDot is never parsed — it is constructed programmatically.
        // Verify it encodes to exactly ".\r\n" (not ".\r\n\r\n").
        k9::assert_equal!(Command::DataDot.encode(), BString::from(".\r\n"));
    }

    #[test]
    fn test_encode_rset() {
        assert_encode("RSET", "RSET\r\n");
    }

    #[test]
    fn test_encode_quit() {
        assert_encode("QUIT", "QUIT\r\n");
    }

    #[test]
    fn test_encode_starttls() {
        assert_encode("STARTTLS", "STARTTLS\r\n");
    }

    #[test]
    fn test_encode_mail_from_path() {
        assert_encode(
            "MAIL FROM:<user@example.com>",
            "MAIL FROM:<user@example.com>\r\n",
        );
    }

    #[test]
    fn test_encode_mail_from_null_sender() {
        assert_encode("MAIL FROM:<>", "MAIL FROM:<>\r\n");
    }

    #[test]
    fn test_encode_mail_from_params() {
        assert_encode(
            "MAIL FROM:<user@example.com> SIZE=1000 BODY=8BITMIME",
            "MAIL FROM:<user@example.com> SIZE=1000 BODY=8BITMIME\r\n",
        );
    }

    #[test]
    fn test_encode_mail_from_ipv4() {
        assert_encode(
            "MAIL FROM:<user@[10.0.0.1]>",
            "MAIL FROM:<user@[10.0.0.1]>\r\n",
        );
    }

    #[test]
    fn test_encode_mail_from_ipv6() {
        assert_encode(
            "MAIL FROM:<user@[IPv6:::1]>",
            "MAIL FROM:<user@[IPv6:::1]>\r\n",
        );
    }

    #[test]
    fn test_encode_rcpt_to_path() {
        assert_encode(
            "RCPT TO:<user@example.com>",
            "RCPT TO:<user@example.com>\r\n",
        );
    }

    #[test]
    fn test_encode_rcpt_to_postmaster() {
        // Canonical encoding is capital-P Postmaster; parsing is case-insensitive
        let cmd = Command::RcptTo {
            address: ForwardPath::Postmaster,
            parameters: vec![],
        };
        k9::assert_equal!(cmd.encode(), BString::from("RCPT TO:<Postmaster>\r\n"));
        k9::assert_equal!(
            unwrapper(Command::parse(cmd.encode())),
            MaybePartialCommand::Full(cmd)
        );
    }

    #[test]
    fn test_encode_rcpt_to_params() {
        assert_encode(
            "RCPT TO:<user@example.com> NOTIFY=SUCCESS",
            "RCPT TO:<user@example.com> NOTIFY=SUCCESS\r\n",
        );
    }

    #[test]
    fn test_encode_auth_no_response() {
        assert_encode("AUTH PLAIN", "AUTH PLAIN\r\n");
    }

    #[test]
    fn test_encode_auth_with_response() {
        assert_encode("AUTH PLAIN dXNlcjpwYXNz", "AUTH PLAIN dXNlcjpwYXNz\r\n");
    }

    #[test]
    fn test_encode_xclient_single() {
        assert_encode(
            "XCLIENT NAME=foo.example.com",
            "XCLIENT NAME=foo.example.com\r\n",
        );
    }

    #[test]
    fn test_encode_xclient_multiple() {
        assert_encode(
            "XCLIENT NAME=foo.example.com ADDR=10.0.0.1",
            "XCLIENT NAME=foo.example.com ADDR=10.0.0.1\r\n",
        );
    }

    #[test]
    fn test_encode_xclient_xtext_roundtrip() {
        // '+40' in wire form decodes to '@'.
        // '@' (ASCII 64) is a valid xchar (range 33-126, excl. '+' and '='),
        // so it is NOT re-encoded as '+40' — it passes through unchanged.
        assert_encode(
            "XCLIENT NAME=user+40example.com",
            "XCLIENT NAME=user@example.com\r\n",
        );
    }

    #[test]
    fn test_encode_unknown() {
        assert_encode("FOOBAR some args", "FOOBAR some args\r\n");
    }

    #[test]
    fn test_encode_unknown_bare_verb() {
        assert_encode("FOOBAR", "FOOBAR\r\n");
    }

    #[test]
    fn test_encode_mail_from_source_route_dropped() {
        // A command parsed with a non-empty at_domain_list encodes without the
        // source route (RFC 5321 says SHOULD NOT generate).  The re-parsed
        // result therefore has an empty at_domain_list.
        let parsed = unwrapper(Command::parse("MAIL FROM:<@route.example.com:user@host>"));
        let cmd = match parsed {
            MaybePartialCommand::Full(c) => c,
            other => panic!("expected Full, got {other:?}"),
        };
        let encoded = cmd.encode();
        k9::assert_equal!(encoded, BString::from("MAIL FROM:<user@host>\r\n"));
        // Re-parsing gives back the same mailbox but with an empty source route
        let expected = Mailbox {
            local_part: "user".into(),
            domain: Domain::DomainName("host".parse().unwrap()),
        };
        k9::assert_equal!(
            unwrapper(Command::parse(encoded)),
            MaybePartialCommand::Full(Command::MailFrom {
                address: expected.into(),
                parameters: vec![],
            })
        );
    }

    // ------------------------------------------------------------------
    // RFC 6531 / non-ASCII tests
    // ------------------------------------------------------------------

    /// Helper: parse a Full command from a known-good string.
    fn parse_full(input: &str) -> Command {
        match unwrapper(Command::parse(input)) {
            MaybePartialCommand::Full(c) => c,
            other => panic!("expected Full, got {other:?}"),
        }
    }

    // --- Non-ASCII local parts ---

    #[test]
    fn test_mail_from_utf8_local_part() {
        // ü = U+00FC, UTF-8 encoding [0xc3, 0xbc]
        let cmd = parse_full("MAIL FROM:<ü@example.com>");
        let mailbox = match cmd {
            Command::MailFrom {
                address: ReversePath::Path(path),
                ..
            } => path.mailbox,
            other => panic!("unexpected {other:?}"),
        };
        k9::assert_equal!(mailbox.local_part, String::from("ü"));
        k9::assert_equal!(
            mailbox.domain,
            Domain::DomainName("example.com".parse().unwrap())
        );
    }

    #[test]
    fn test_mail_from_utf8_local_part_roundtrip() {
        // Non-ASCII local parts survive encode → parse unchanged because
        // encode_mailbox copies the raw bytes and parse stores them verbatim.
        let input = "MAIL FROM:<ü@example.com>";
        let cmd = parse_full(input);
        k9::assert_equal!(
            unwrapper(Command::parse(cmd.encode())),
            MaybePartialCommand::Full(cmd)
        );
    }

    #[test]
    fn test_mail_from_quoted_utf8_local_part() {
        // UTF-8 characters are valid inside a quoted-string local part (RFC 6532).
        let cmd = parse_full("MAIL FROM:<\"ü\"@example.com>");
        let mailbox = match cmd {
            Command::MailFrom {
                address: ReversePath::Path(path),
                ..
            } => path.mailbox,
            other => panic!("unexpected {other:?}"),
        };
        // local_part stores the string including the surrounding quotes
        k9::assert_equal!(mailbox.local_part, String::from("\"ü\""));
    }

    #[test]
    fn test_rcpt_to_utf8_local_part() {
        // CJK characters in the local part (RFC 6531 EAI)
        let cmd = parse_full("RCPT TO:<用户@example.com>");
        let mailbox = match cmd {
            Command::RcptTo {
                address: ForwardPath::Path(path),
                ..
            } => path.mailbox,
            other => panic!("unexpected {other:?}"),
        };
        k9::assert_equal!(mailbox.local_part, String::from("用户"));
    }

    // --- U-label (non-ASCII) domain names ---

    #[test]
    fn test_mail_from_u_label_domain() {
        // münchen.de — stored as normalized ASCII/punycode form
        let cmd = parse_full("MAIL FROM:<user@münchen.de>");
        let domain = match cmd {
            Command::MailFrom {
                address: ReversePath::Path(path),
                ..
            } => path.mailbox.domain,
            other => panic!("unexpected {other:?}"),
        };
        match domain {
            Domain::DomainName(ref s) => {
                k9::assert_equal!(s.as_str(), "xn--mnchen-3ya.de");
            }
            other => panic!("unexpected domain variant {other:?}"),
        }
    }

    #[test]
    fn test_mail_from_u_label_domain_encode() {
        // encode_domain normalises U-labels to their ASCII/punycode form.
        // This is intentional: punycode is always safe for wire transmission.
        let cmd = parse_full("MAIL FROM:<user@münchen.de>");
        k9::assert_equal!(
            cmd.encode(),
            BString::from("MAIL FROM:<user@xn--mnchen-3ya.de>\r\n")
        );
    }

    #[test]
    fn test_ehlo_u_label_domain() {
        // DomainString::as_str() returns the normalized ASCII/punycode form
        let cmd = parse_full("EHLO münchen.de");
        match cmd {
            Command::Ehlo(Domain::DomainName(ref s)) => {
                k9::assert_equal!(s.as_str(), "xn--mnchen-3ya.de");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_ehlo_u_label_domain_encode() {
        // encode_domain normalises to punycode for EHLO as well
        let cmd = parse_full("EHLO münchen.de");
        k9::assert_equal!(cmd.encode(), BString::from("EHLO xn--mnchen-3ya.de\r\n"));
    }

    // --- Non-ASCII ESMTP values (RFC 6531 §3.3: esmtp-value =/ UTF8-non-ASCII) ---

    #[test]
    fn test_esmtp_value_utf8() {
        // A parameter value containing a non-ASCII UTF-8 character (ü = U+00FC)
        let cmd = parse_full("MAIL FROM:<u@h> PARAM=valüe");
        let params = match cmd {
            Command::MailFrom { parameters, .. } => parameters,
            other => panic!("unexpected {other:?}"),
        };
        k9::assert_equal!(params.len(), 1);
        k9::assert_equal!(params[0].name, "PARAM");
        k9::assert_equal!(params[0].value, Some("valüe".to_string()));
    }

    #[test]
    fn test_esmtp_value_utf8_roundtrip() {
        // Non-ASCII ESMTP values are stored and re-encoded verbatim (raw bytes).
        let input = "MAIL FROM:<u@h> PARAM=valüe";
        let cmd = parse_full(input);
        k9::assert_equal!(
            cmd.encode(),
            BString::from("MAIL FROM:<u@h> PARAM=valüe\r\n")
        );
        k9::assert_equal!(
            unwrapper(Command::parse(cmd.encode())),
            MaybePartialCommand::Full(parse_full(input))
        );
    }

    #[test]
    fn test_esmtp_value_utf8_only() {
        // A value consisting entirely of non-ASCII UTF-8 characters parses successfully
        let cmd = parse_full("MAIL FROM:<u@h> X=ünïcödé");
        let params = match cmd {
            Command::MailFrom { parameters, .. } => parameters,
            other => panic!("unexpected {other:?}"),
        };
        k9::assert_equal!(params[0].name, "X");
        k9::assert_equal!(params[0].value, Some("ünïcödé".to_string()));
    }

    // --- Fallible conversion tests for MailPath ---

    #[test]
    fn test_reverse_path_null_sender_try_into_mailpath_err() {
        use core::convert::TryInto;
        let null_sender = ReversePath::NullSender;
        let err: &'static str =
            <ReversePath as TryInto<MailPath>>::try_into(null_sender).unwrap_err();
        k9::assert_equal!(err, "Cannot convert NullSender to MailPath");
    }

    #[test]
    fn test_forward_path_postmaster_try_into_mailpath_err() {
        use core::convert::TryInto;
        let postmaster = ForwardPath::Postmaster;
        let err: &'static str =
            <ForwardPath as TryInto<MailPath>>::try_into(postmaster).unwrap_err();
        k9::assert_equal!(err, "Cannot convert Postmaster to MailPath");
    }

    #[test]
    fn test_reverse_path_try_from_null_sender_err() {
        let null_sender = ReversePath::NullSender;
        let err = MailPath::try_from(null_sender).unwrap_err();
        k9::assert_equal!(err, "Cannot convert NullSender to MailPath");
    }

    #[test]
    fn test_forward_path_try_from_postmaster_err() {
        let postmaster = ForwardPath::Postmaster;
        let err = MailPath::try_from(postmaster).unwrap_err();
        k9::assert_equal!(err, "Cannot convert Postmaster to MailPath");
    }

    // --- Fallible conversion tests for EnvelopeAddress ---

    #[test]
    fn test_envelope_address_try_from_mailbox() {
        let mailbox = Mailbox {
            local_part: String::from("user"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let addr = EnvelopeAddress::from(mailbox);
        match addr {
            EnvelopeAddress::Path(path) => {
                k9::assert_equal!(path.mailbox.local_part(), "user");
                k9::assert_equal!(
                    path.mailbox.domain,
                    Domain::DomainName("example.com".parse().unwrap())
                );
            }
            _ => panic!("Expected Path variant"),
        }
    }

    #[test]
    fn test_envelope_address_try_from_null_err() {
        let addr = EnvelopeAddress::Null;
        let err = Mailbox::try_from(addr).unwrap_err();
        k9::assert_equal!(err, "Cannot convert Null to Mailbox");
    }

    #[test]
    fn test_envelope_address_try_from_postmaster_err() {
        let addr = EnvelopeAddress::Postmaster;
        let err = Mailbox::try_from(addr).unwrap_err();
        k9::assert_equal!(err, "Cannot convert Postmaster to Mailbox");
    }

    #[test]
    fn test_envelope_address_try_from_null_to_reverse_path() {
        let addr = EnvelopeAddress::Null;
        let result = ReversePath::try_from(addr).unwrap();
        k9::assert_equal!(result, ReversePath::NullSender);
    }

    #[test]
    fn test_envelope_address_try_from_postmaster_to_forward_path() {
        let addr = EnvelopeAddress::Postmaster;
        let result = ForwardPath::try_from(addr).unwrap();
        k9::assert_equal!(result, ForwardPath::Postmaster);
    }

    // --- Fallible conversion tests for MailPath ---

    #[test]
    fn test_envelope_address_try_into_mailpath_null_err() {
        let addr = EnvelopeAddress::Null;
        let err = MailPath::try_from(addr).unwrap_err();
        k9::assert_equal!(err, "Cannot convert Null to MailPath");
    }

    #[test]
    fn test_envelope_address_try_into_mailpath_postmaster_err() {
        let addr = EnvelopeAddress::Postmaster;
        let err = MailPath::try_from(addr).unwrap_err();
        k9::assert_equal!(err, "Cannot convert Postmaster to MailPath");
    }

    #[test]
    fn test_reverse_path_try_into_forward_path_null_sender_err() {
        let rp = ReversePath::NullSender;
        let err = ForwardPath::try_from(rp).unwrap_err();
        k9::assert_equal!(err, "Cannot convert NullSender to ForwardPath");
    }

    #[test]
    fn test_forward_path_try_into_reverse_path_postmaster_err() {
        let fp = ForwardPath::Postmaster;
        let err = ReversePath::try_from(fp).unwrap_err();
        k9::assert_equal!(err, "Cannot convert Postmaster to ReversePath");
    }

    // --- Infallible conversion tests for Mailbox ---

    #[test]
    fn test_mailbox_into_mailpath() {
        let mailbox = Mailbox {
            local_part: String::from("user"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let path: MailPath = mailbox.into();
        assert!(path.at_domain_list.is_empty());
        k9::assert_equal!(path.mailbox.local_part(), "user");
    }

    #[test]
    fn test_mailbox_into_envelope_address() {
        let mailbox = Mailbox {
            local_part: String::from("user"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let addr: EnvelopeAddress = mailbox.into();
        match addr {
            EnvelopeAddress::Path(path) => {
                k9::assert_equal!(path.mailbox.local_part(), "user");
            }
            _ => panic!("Expected Path variant"),
        }
    }

    #[test]
    fn test_mailbox_into_reverse_path() {
        let mailbox = Mailbox {
            local_part: String::from("user"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let path: ReversePath = mailbox.into();
        match path {
            ReversePath::Path(p) => {
                k9::assert_equal!(p.mailbox.local_part(), "user");
            }
            _ => panic!("Expected Path variant"),
        }
    }

    #[test]
    fn test_mailbox_into_forward_path() {
        let mailbox = Mailbox {
            local_part: String::from("user"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let path: ForwardPath = mailbox.into();
        match path {
            ForwardPath::Path(p) => {
                k9::assert_equal!(p.mailbox.local_part(), "user");
            }
            _ => panic!("Expected Path variant"),
        }
    }

    // --- Round-trip conversion tests ---

    #[test]
    fn test_mailbox_roundtrip() {
        let original = Mailbox {
            local_part: String::from("user"),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let path: MailPath = original.clone().into();
        let mailbox: Mailbox = path.mailbox.into();
        k9::assert_equal!(original.local_part(), mailbox.local_part());
        k9::assert_equal!(original.domain, mailbox.domain);
    }

    #[test]
    fn test_reverse_path_to_forward_path() {
        let path = MailPath {
            at_domain_list: vec!["route.com".into()],
            mailbox: Mailbox {
                local_part: String::from("user"),
                domain: Domain::DomainName("example.com".parse().unwrap()),
            },
        };
        let rp = ReversePath::Path(path.clone());
        let fp: ForwardPath = rp.try_into().unwrap();
        match fp {
            ForwardPath::Path(p) => {
                k9::assert_equal!(p.mailbox.local_part(), "user");
            }
            _ => panic!("Expected Path variant"),
        }
    }

    // --- Debug tests for MailPath ---

    #[test]
    fn test_mailpath_debug_simple() {
        let mailbox = Mailbox {
            local_part: "someone".into(),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let path: MailPath = mailbox.into();
        let debug_str = format!("{:?}", path);
        k9::assert_equal!(debug_str, r#"MailPath("someone@example.com")"#);
    }

    #[test]
    fn test_mailpath_debug_with_at_domain_list() {
        let path = MailPath {
            at_domain_list: vec!["route.example.com".into()],
            mailbox: Mailbox {
                local_part: "user".into(),
                domain: Domain::DomainName("host".parse().unwrap()),
            },
        };
        let debug_str = format!("{:?}", path);
        k9::assert_equal!(debug_str, r#"MailPath("@route.example.com:user@host")"#);
    }

    #[test]
    fn test_mailpath_debug_multiple_at_domains() {
        let path = MailPath {
            at_domain_list: vec!["hosta.int".into(), "jkl.org".into()],
            mailbox: Mailbox {
                local_part: "userc".into(),
                domain: Domain::DomainName("d.bar.org".parse().unwrap()),
            },
        };
        let debug_str = format!("{:?}", path);
        k9::assert_equal!(
            debug_str,
            r#"MailPath("@hosta.int,@jkl.org:userc@d.bar.org")"#
        );
    }

    #[test]
    fn test_mailpath_debug_non_ascii_utf8() {
        // UTF-8 character ü (U+00FC) which is non-ASCII but valid UTF-8
        // should be preserved as-is in debug output
        let mailbox = Mailbox {
            local_part: "üser".into(),
            domain: Domain::DomainName("example.com".parse().unwrap()),
        };
        let path: MailPath = mailbox.into();
        let debug_str = format!("{:?}", path);
        // ü is valid UTF-8, should appear as-is
        k9::assert_equal!(debug_str, r#"MailPath("üser@example.com")"#);
    }

    #[test]
    fn test_mailpath_debug_with_ipv4() {
        let mailbox = Mailbox {
            local_part: "user".into(),
            domain: Domain::V4("10.0.0.1".parse().unwrap()),
        };
        let path: MailPath = mailbox.into();
        let debug_str = format!("{:?}", path);
        // IPv4 literals must be in brackets per RFC 5321
        k9::assert_equal!(debug_str, r#"MailPath("user@[10.0.0.1]")"#);
    }

    #[test]
    fn test_mailpath_debug_with_ipv6() {
        let mailbox = Mailbox {
            local_part: "user".into(),
            domain: Domain::V6("::1".parse().unwrap()),
        };
        let path: MailPath = mailbox.into();
        let debug_str = format!("{:?}", path);
        // IPv6 literals must be in brackets with IPv6: prefix per RFC 5321
        k9::assert_equal!(debug_str, r#"MailPath("user@[IPv6:::1]")"#);
    }

    #[test]
    fn test_mailpath_debug_with_tagged_literal() {
        let mailbox = Mailbox {
            local_part: "user".into(),
            domain: Domain::Tagged("future:something".into()),
        };
        let path: MailPath = mailbox.into();
        let debug_str = format!("{:?}", path);
        // Tagged literals must be in brackets per RFC 5321
        k9::assert_equal!(debug_str, r#"MailPath("user@[future:something]")"#);
    }
    #[test]
    fn test_envelope_address_debug_null() {
        let addr = EnvelopeAddress::Null;
        let debug_str = format!("{:?}", addr);
        k9::assert_equal!(debug_str, "<>");
    }

    #[test]
    fn test_envelope_address_debug_postmaster() {
        let addr = EnvelopeAddress::Postmaster;
        let debug_str = format!("{:?}", addr);
        k9::assert_equal!(debug_str, "<Postmaster>");
    }

    #[test]
    fn test_envelope_address_debug_path_with_non_ascii_utf8() {
        // UTF-8 character ü (U+00FC) which is non-ASCII but valid UTF-8
        // should be preserved as-is in debug output
        let path = MailPath {
            at_domain_list: vec!["example.com".into()],
            mailbox: Mailbox {
                local_part: String::from("üser"),
                domain: Domain::DomainName("example.com".parse().unwrap()),
            },
        };
        let addr = EnvelopeAddress::Path(path);
        let debug_str = format!("{:?}", addr);
        // ü is valid UTF-8, should appear as-is
        k9::assert_equal!(debug_str, r#"<@example.com:üser@example.com>"#);
    }
}
