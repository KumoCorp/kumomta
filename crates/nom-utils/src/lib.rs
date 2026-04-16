use bstr::{BStr, ByteSlice};
use hickory_resolver::Name;
use nom::branch::alt;
use nom::bytes::complete::{take_while1, take_while_m_n};
use nom::combinator::{map_res, opt, recognize};
use nom::error::{context, ContextError, ErrorKind, FromExternalError, ParseError as _};
use nom::multi::{many0, many1};
use nom::sequence::pair;
use nom::{Input, Parser as _};
use nom_locate::LocatedSpan;
use std::fmt::{self, Debug, Write};
use std::hash::Hash;
use std::marker::PhantomData;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

pub type Span<'a> = LocatedSpan<&'a [u8]>;
pub type IResult<'a, A, B> = nom::IResult<A, B, ParseError<Span<'a>>>;

pub fn make_span(s: &'_ [u8]) -> Span<'_> {
    Span::new(s)
}

/// Like nom::bytes::complete::tag, except that we print what the tag
/// was expecting if there was an error.
/// I feel like this should be the default behavior TBH.
pub fn tag<E>(tag: &'static str) -> TagParser<E> {
    TagParser {
        tag,
        no_case: false,
        e: PhantomData,
    }
}

pub fn tag_no_case<E>(tag: &'static str) -> TagParser<E> {
    TagParser {
        tag,
        no_case: true,
        e: PhantomData,
    }
}

/// Struct to support displaying better errors for tag()
pub struct TagParser<E> {
    tag: &'static str,
    no_case: bool,
    e: PhantomData<E>,
}

/// All this fuss to show what we expected for the TagParser impl
impl<I, Error: nom::error::ParseError<I> + nom::error::FromExternalError<I, String>> nom::Parser<I>
    for TagParser<Error>
where
    I: nom::Input + nom::Compare<&'static str> + nom::AsBytes,
{
    type Output = I;
    type Error = Error;

    fn process<OM: nom::OutputMode>(
        &mut self,
        i: I,
    ) -> nom::PResult<OM, I, Self::Output, Self::Error> {
        use nom::error::ErrorKind;
        use nom::{CompareResult, Err, Mode};

        let tag_len = self.tag.input_len();

        let compare_result = if self.no_case {
            i.compare_no_case(self.tag)
        } else {
            i.compare(self.tag)
        };

        match compare_result {
            CompareResult::Ok => Ok((i.take_from(tag_len), OM::Output::bind(|| i.take(tag_len)))),
            CompareResult::Incomplete => Err(Err::Error(OM::Error::bind(|| {
                Error::from_external_error(
                    i,
                    ErrorKind::Fail,
                    format!(
                        "expected \"{}\" but ran out of input",
                        self.tag.escape_debug()
                    ),
                )
            }))),

            CompareResult::Error => {
                let available = i.take(i.input_len().min(tag_len));
                Err(Err::Error(OM::Error::bind(|| {
                    Error::from_external_error(
                        i,
                        ErrorKind::Fail,
                        format!(
                            "expected \"{}\" but found {:?}",
                            self.tag.escape_debug(),
                            BStr::new(available.as_bytes())
                        ),
                    )
                })))
            }
        }
    }
}

#[derive(Debug)]
pub enum ParseErrorKind {
    Context(&'static str),
    Char(char),
    Nom(ErrorKind),
    External { kind: ErrorKind, reason: String },
}

#[derive(Debug)]
pub struct ParseError<I: Debug> {
    pub errors: Vec<(I, ParseErrorKind)>,
}

impl<I: Debug> ContextError<I> for ParseError<I> {
    fn add_context(input: I, ctx: &'static str, mut other: Self) -> Self {
        other.errors.push((input, ParseErrorKind::Context(ctx)));
        other
    }
}

impl<I: Debug> nom::error::ParseError<I> for ParseError<I> {
    fn from_error_kind(input: I, kind: ErrorKind) -> Self {
        Self {
            errors: vec![(input, ParseErrorKind::Nom(kind))],
        }
    }

    fn append(input: I, kind: ErrorKind, mut other: Self) -> Self {
        other.errors.push((input, ParseErrorKind::Nom(kind)));
        other
    }

    fn from_char(input: I, c: char) -> Self {
        Self {
            errors: vec![(input, ParseErrorKind::Char(c))],
        }
    }
}

impl<I: Debug, E: std::fmt::Display> nom::error::FromExternalError<I, E> for ParseError<I> {
    fn from_external_error(input: I, kind: ErrorKind, err: E) -> Self {
        Self {
            errors: vec![(
                input,
                ParseErrorKind::External {
                    kind,
                    reason: format!("{err:#}"),
                },
            )],
        }
    }
}

pub fn make_context_error<S: Into<String>>(
    input: Span<'_>,
    reason: S,
) -> nom::Err<ParseError<Span<'_>>> {
    nom::Err::Error(ParseError {
        errors: vec![(
            input,
            ParseErrorKind::External {
                kind: nom::error::ErrorKind::Fail,
                reason: reason.into(),
            },
        )],
    })
}

pub fn explain_nom(input: Span, err: nom::Err<ParseError<Span<'_>>>) -> String {
    match err {
        nom::Err::Error(e) => {
            let mut result = String::new();
            let mut lines_shown = vec![];

            for (span, kind) in e.errors.iter() {
                if input.is_empty() {
                    match kind {
                        ParseErrorKind::Char(c) => {
                            write!(&mut result, "Error expected '{c}', got empty input\n\n")
                        }
                        ParseErrorKind::Context(s) => {
                            write!(&mut result, "Error in {s}, got empty input\n\n")
                        }
                        ParseErrorKind::External { kind, reason } => {
                            write!(&mut result, "Error {reason} {kind:?}, got empty input\n\n")
                        }
                        ParseErrorKind::Nom(e) => {
                            write!(&mut result, "Error in {e:?}, got empty input\n\n")
                        }
                    }
                    .ok();
                    continue;
                }

                let line_number = span.location_line();
                let input_line = span.get_line_beginning();
                // Remap \t in particular, because it can render as multiple
                // columns and defeat the column number calculation provided
                // by the Span type
                let mut line = String::new();
                for (start, end, c) in input_line.char_indices() {
                    let c = match c {
                        '\t' => '\u{2409}',
                        '\r' => '\u{240d}',
                        '\n' => '\u{240a}',
                        c => c,
                    };

                    if c == std::char::REPLACEMENT_CHARACTER {
                        let bytes = &input_line[start..end];
                        for b in bytes.iter() {
                            line.push_str(&format!("\\x{b:02X}"));
                        }
                    } else {
                        line.push(c);
                    }
                }

                let column = span.get_utf8_column();

                lines_shown.push(line_number);

                let mut caret = " ".repeat(column.saturating_sub(1));
                caret.push('^');
                for _ in 1..span.fragment().len() {
                    caret.push('_')
                }

                match kind {
                    ParseErrorKind::Char(expected) => {
                        if let Some(actual) = span.fragment().chars().next() {
                            write!(
                                &mut result,
                                "Error at line {line_number}:\n\
                                    {line}\n\
                                    {caret}\n\
                                    expected '{expected}', found {actual}\n\n",
                            )
                        } else {
                            write!(
                                &mut result,
                                "Error at line {line_number}:\n\
                                    {line}\n\
                                    {caret}\n\
                                    expected '{expected}', got end of input\n\n",
                            )
                        }
                    }
                    ParseErrorKind::Context(context) => {
                        write!(&mut result, "while parsing {context}\n")
                    }
                    ParseErrorKind::External { kind: _, reason } => {
                        write!(
                            &mut result,
                            "Error at line {line_number}, {reason}:\n\
                                {line}\n\
                                {caret}\n\n",
                        )
                    }
                    ParseErrorKind::Nom(nom_err) => {
                        write!(
                            &mut result,
                            "Error at line {line_number}, in {nom_err:?}:\n\
                                {line}\n\
                                {caret}\n\n",
                        )
                    }
                }
                .ok();
            }
            result
        }
        _ => format!("{err:#}"),
    }
}

/// See the following RFCs:
/// * <https://datatracker.ietf.org/doc/html/rfc6531#section-3.3>
/// * <https://datatracker.ietf.org/doc/html/rfc6532#section-3.1>
/// * <https://datatracker.ietf.org/doc/html/rfc3629#section-4>
/// which define a bunch of ABNF, but then caps it off with:
/// > The authoritative definition of UTF-8 is in [UNICODE].  This
/// > grammar is believed to describe the same thing Unicode describes, but
/// > does not claim to be authoritative.  Implementors are urged to rely
/// > on the authoritative source, rather than on this ABNF.
pub fn utf8_non_ascii(input: Span) -> IResult<Span, Span> {
    use nom::Err;

    match input.char_indices().next() {
        Some((start, end, c)) => {
            let len = end - start;
            if c as u32 <= 0x7f {
                // It's ASCII, therefore doesn't match as utf8_non_ascii
                return Err(Err::Error(ParseError::from_error_kind(
                    input,
                    ErrorKind::Fail,
                )));
            }
            let slice = &input[start..end];
            if c == std::char::REPLACEMENT_CHARACTER {
                let mut verify = [0u8; 4];
                if slice != c.encode_utf8(&mut verify).as_bytes() {
                    // The original sequence wasn't REPLACEMENT_CHARACTER,
                    // therefore the input is not valid UTF-8
                    return Err(Err::Error(ParseError::from_error_kind(
                        input,
                        ErrorKind::Fail,
                    )));
                }
            }
            // slice is the first UTF-8 character in the input
            Ok((input.take_from(len), input.take(len)))
        }
        None => {
            // There's no input, therefore we cannot match
            Err(Err::Error(ParseError::from_error_kind(
                input,
                ErrorKind::Eof,
            )))
        }
    }
}

fn snum(input: Span) -> IResult<Span, Span> {
    take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()).parse(input)
}

pub fn ipv4_address(input: Span) -> IResult<Span, Ipv4Addr> {
    context(
        "ipv4_address",
        map_res(
            recognize((snum, tag("."), snum, tag("."), snum, tag("."), snum)),
            |matched| {
                let v4str = std::str::from_utf8(&matched).expect("can only be ascii");
                v4str.parse().map_err(|err| {
                    nom::Err::Error(ParseError::from_external_error(
                        input,
                        ErrorKind::Fail,
                        format!("invalid ipv4_address: {err}"),
                    ))
                })
            },
        ),
    )
    .parse(input)
}

pub fn ipv6_address(input: Span) -> IResult<Span, Ipv6Addr> {
    context(
        "ipv6_address",
        map_res(
            take_while1(|c: u8| c.is_ascii_hexdigit() || c == b':' || c == b'.'),
            |matched: Span| {
                let v6str = std::str::from_utf8(&matched).expect("can only be ascii");
                v6str.parse().map_err(|err| {
                    nom::Err::Error(ParseError::from_external_error(
                        input,
                        ErrorKind::Fail,
                        format!("invalid ipv6_address: {err}"),
                    ))
                })
            },
        ),
    )
    .parse(input)
}

/// A validated DNS domain name, stored in normalized (ASCII/punycode) form.
/// The original wire-format string (which may have been a UTF-8 U-label)
/// is not preserved; only the IDNA-normalized A-label form is kept.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DomainString(String);

impl fmt::Display for DomainString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl DomainString {
    pub fn name(&self) -> Name {
        Name::from_str_relaxed(&self.0)
            .expect("cannot construct DomainString with an invalid domain name")
    }

    /// Returns a reference to the normalized (ASCII/punycode) domain string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DomainString {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let name = Name::from_str_relaxed(s)?;
        Ok(Self(name.to_ascii()))
    }
}

impl From<DomainString> for Name {
    fn from(val: DomainString) -> Self {
        val.name()
    }
}

impl From<&DomainString> for Name {
    fn from(val: &DomainString) -> Self {
        val.name()
    }
}

/// `let-dig = ALPHA / DIGIT / UTF8-non-ASCII`
fn let_dig(input: Span) -> IResult<Span, Span> {
    recognize(alt((
        take_while_m_n(1, 1, |c: u8| c.is_ascii_alphanumeric()),
        utf8_non_ascii,
    )))
    .parse(input)
}

/// `ldh-str = *( ALPHA / DIGIT / "-" / UTF8-non-ASCII )`  (one or more)
///
/// As an extension to the mail RFCs, we allow for underscore
/// in domain names, as those are a commonly deployed name, despite it
/// being in violation of the DNS RFCs.
fn ldh_str(input: Span) -> IResult<Span, Span> {
    recognize(many1(alt((
        take_while_m_n(1, 1, |c: u8| {
            c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
        }),
        utf8_non_ascii,
    ))))
    .parse(input)
}

/// `sub-domain = let-dig [ ldh-str ]`
fn sub_domain(input: Span) -> IResult<Span, Span> {
    recognize(pair(let_dig, opt(ldh_str))).parse(input)
}

/// `domain = sub-domain *( "." sub-domain )`
pub fn domain_name(input: Span) -> IResult<Span, DomainString> {
    context(
        "domain-name",
        map_res(
            recognize(pair(sub_domain, many0(pair(tag("."), sub_domain)))),
            |matched: Span| match std::str::from_utf8(&matched) {
                Ok(s) => s.parse().map_err(|err| {
                    nom::Err::Error(ParseError::from_external_error(
                        input,
                        ErrorKind::Fail,
                        format!("invalid domain name: {err}"),
                    ))
                }),
                Err(err) => Err(nom::Err::Error(ParseError::from_external_error(
                    input,
                    ErrorKind::Fail,
                    format!("invalid domain name: {err}"),
                ))),
            },
        ),
    )
    .parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv4_parse() {
        // ipv4_address should parse valid IPv4 addresses
        let (_, addr) = ipv4_address(make_span(b"192.168.1.1")).unwrap();
        k9::assert_equal!(addr, Ipv4Addr::new(192, 168, 1, 1));
    }

    #[test]
    fn test_ipv6_parse() {
        // ipv6_address should parse valid IPv6 addresses,
        // and different representations of the same address should be equal
        let (_, v6a) = ipv6_address(make_span(b"2001:0db8:0000:0000:0000:0000:0000:0001")).unwrap();
        let (_, v6b) = ipv6_address(make_span(b"2001:db8::1")).unwrap();
        k9::assert_equal!(v6a, v6b);
    }

    #[test]
    fn test_domain_string_partial_eq() {
        // DomainString should compare equal if they normalize to the same domain
        let d1 = DomainString::from_str("EXAMPLE.COM").unwrap();
        let d2 = DomainString::from_str("example.com").unwrap();

        assert_eq!(d1, d2);
    }

    #[test]
    fn test_domain_string_partial_eq_idna() {
        // DomainString should compare equal after IDNA normalization
        let d1 = DomainString::from_str("münchen.de").unwrap();
        let d2 = DomainString::from_str("xn--mnchen-3ya.de").unwrap();

        assert_eq!(d1, d2);
    }
}
