use crate::headermap::EncodeHeaderValue;
use crate::{MailParsingError, Result, SharedString};
use charset::Charset;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while1};
use nom::character::complete::{char, satisfy};
use nom::combinator::{all_consuming, map, opt, recognize};
use nom::error::{context, ContextError, VerboseError};
use nom::multi::{many0, many1, separated_list1};
use nom::sequence::{delimited, preceded, separated_pair, terminated, tuple};
use nom_locate::LocatedSpan;
use nom_tracable::{tracable_parser, TracableInfo};
use pest::iterators::{Pair, Pairs};
use pest::Parser as _;
use pest_derive::*;

pub(crate) type Span<'a> = LocatedSpan<&'a str, TracableInfo>;
type IResult<'a, A, B> = nom::IResult<A, B, VerboseError<Span<'a>>>;

impl MailParsingError {
    pub fn from_nom(input: Span, err: nom::Err<VerboseError<Span<'_>>>) -> Self {
        use nom::error::VerboseErrorKind;
        use std::fmt::Write;
        match err {
            nom::Err::Error(e) => {
                let mut result = String::new();
                for (i, (span, kind)) in e.errors.iter().enumerate() {
                    if input.is_empty() {
                        match kind {
                            VerboseErrorKind::Char(c) => {
                                write!(&mut result, "{i}: expected '{c}', got empty input\n\n")
                            }
                            VerboseErrorKind::Context(s) => {
                                write!(&mut result, "{i}: in {s}, got empty input\n\n")
                            }
                            VerboseErrorKind::Nom(e) => {
                                write!(&mut result, "{i}: in {e:?}, got empty input\n\n")
                            }
                        }
                        .ok();
                        continue;
                    }

                    let line_number = span.location_line();
                    let line = std::str::from_utf8(span.get_line_beginning())
                        .unwrap_or("<INVALID: line slice is not utf8!>");
                    let column = span.get_utf8_column();

                    match kind {
                        VerboseErrorKind::Char(expected) => {
                            if let Some(actual) = span.fragment().chars().next() {
                                write!(
                                    &mut result,
                                    "{i}: at line {line_number}:\n\
                                    {line}\n\
                                    {caret:>column$}\n\
                                    expected '{expected}', found {actual}\n\n",
                                    caret = '^',
                                )
                            } else {
                                write!(
                                    &mut result,
                                    "{i}: at line {line_number}:\n\
                                    {line}\n\
                                    {caret:>column$}\n\
                                    expected '{expected}', got end of input\n\n",
                                    caret = '^',
                                )
                            }
                        }
                        VerboseErrorKind::Context(context) => {
                            write!(
                                &mut result,
                                "{i}: at line {line_number}, in {context}:\n\
                                {line}\n\
                                {caret:>column$}\n\n",
                                caret = '^',
                            )
                        }
                        VerboseErrorKind::Nom(nom_err) => {
                            write!(
                                &mut result,
                                "{i}: at line {line_number}, in {nom_err:?}:\n\
                                {line}\n\
                                {caret:>column$}\n\n",
                                caret = '^',
                            )
                        }
                    }
                    .ok();
                }
                MailParsingError::HeaderParse(result)
            }
            _ => MailParsingError::HeaderParse(format!("{err:#}")),
        }
    }
}

fn make_context_error<'a>(
    input: Span<'a>,
    context: &'static str,
) -> nom::Err<VerboseError<Span<'a>>> {
    let err = nom::error::make_error(input, nom::error::ErrorKind::Fail);

    nom::Err::Error(VerboseError::add_context(input, context, err))
}

fn make_span(s: &str) -> Span {
    let info = TracableInfo::new()
        .forward(true)
        .backward(true)
        .parser_width(42);
    LocatedSpan::new_extra(s, info)
}

fn is_utf8_non_ascii(c: char) -> bool {
    let c = c as u32;
    c == 0 || c >= 0x80
}

// ctl = { '\u{00}'..'\u{1f}' | "\u{7f}" }
fn is_ctl(c: char) -> bool {
    match c {
        '\u{00}'..='\u{1f}' | '\u{7f}' => true,
        _ => false,
    }
}

// char = { '\u{01}'..'\u{7f}' }
fn is_char(c: char) -> bool {
    match c {
        '\u{01}'..='\u{ff}' => true,
        _ => false,
    }
}

fn is_especial(c: char) -> bool {
    match c {
        '(' | ')' | '<' | '>' | '@' | ',' | ';' | ':' | '/' | '[' | ']' | '?' | '.' | '=' => true,
        _ => false,
    }
}

fn is_token(c: char) -> bool {
    is_char(c) && c != ' ' && !is_especial(c) && !is_ctl(c)
}

// vchar = { '\u{21}'..'\u{7e}' | utf8_non_ascii }
fn is_vchar(c: char) -> bool {
    let u = c as u32;
    (u >= 0x21 && u <= 0x7e) || is_utf8_non_ascii(c)
}

#[tracable_parser]
fn atext(input: Span) -> IResult<Span, Span> {
    context(
        "atext",
        take_while1(|c| match c {
            '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '/' | '=' | '?' | '^' | '_'
            | '`' | '{' | '|' | '}' | '~' => true,
            c => c.is_ascii_alphanumeric() || is_utf8_non_ascii(c),
        }),
    )(input)
}

fn is_obs_no_ws_ctl(c: char) -> bool {
    match c {
        '\u{01}'..='\u{08}' | '\u{0b}'..='\u{0c}' | '\u{0e}'..='\u{1f}' | '\u{7f}' => true,
        _ => false,
    }
}

fn is_obs_ctext(c: char) -> bool {
    is_obs_no_ws_ctl(c)
}

// ctext = { '\u{21}'..'\u{27}' | '\u{2a}'..'\u{5b}' | '\u{5d}'..'\u{7e}' | obs_ctext | utf8_non_ascii }
fn is_ctext(c: char) -> bool {
    match c {
        '\u{21}'..='\u{27}' | '\u{2a}'..='\u{5b}' | '\u{5d}'..='\u{7e}' => true,
        c => is_obs_ctext(c) || is_utf8_non_ascii(c),
    }
}

// dtext = { '\u{21}'..'\u{5a}' | '\u{5e}'..'\u{7e}' | obs_dtext | utf8_non_ascii }
// obs_dtext = { obs_no_ws_ctl | quoted_pair }
fn is_dtext(c: char) -> bool {
    match c {
        '\u{21}'..='\u{5a}' | '\u{5e}'..='\u{7e}' => true,
        c => is_obs_no_ws_ctl(c) || is_utf8_non_ascii(c),
    }
}

// qtext = { "\u{21}" | '\u{23}'..'\u{5b}' | '\u{5d}'..'\u{7e}' | obs_qtext | utf8_non_ascii }
// obs_qtext = { obs_no_ws_ctl }
fn is_qtext(c: char) -> bool {
    match c {
        '\u{21}' | '\u{23}'..='\u{5b}' | '\u{5d}'..='\u{7e}' => true,
        c => is_obs_no_ws_ctl(c) || is_utf8_non_ascii(c),
    }
}

#[tracable_parser]
fn wsp(input: Span) -> IResult<Span, Span> {
    context("wsp", take_while1(|c| c == ' ' || c == '\t'))(input)
}

#[tracable_parser]
fn newline(input: Span) -> IResult<Span, Span> {
    context("newline", recognize(preceded(opt(char('\r')), char('\n'))))(input)
}

// fws = { ((wsp* ~ "\r"? ~ "\n")* ~ wsp+) | obs_fws }
#[tracable_parser]
fn fws(input: Span) -> IResult<Span, Span> {
    context(
        "fws",
        alt((
            recognize(preceded(many0(preceded(many0(wsp), newline)), many1(wsp))),
            obs_fws,
        )),
    )(input)
}

// obs_fws = { wsp+ ~ ("\r"? ~ "\n" ~ wsp+)* }
#[tracable_parser]
fn obs_fws(input: Span) -> IResult<Span, Span> {
    context(
        "obs_fws",
        recognize(preceded(many1(wsp), preceded(newline, many1(wsp)))),
    )(input)
}

// mailbox_list = { (mailbox ~ ("," ~ mailbox)*) | obs_mbox_list }
#[tracable_parser]
fn mailbox_list(input: Span) -> IResult<Span, MailboxList> {
    let (loc, mailboxes) = context(
        "mailbox_list",
        alt((separated_list1(char(','), mailbox), obs_mbox_list)),
    )(input)?;
    Ok((loc, MailboxList(mailboxes)))
}

// obs_mbox_list = {  ((cfws? ~ ",")* ~ mailbox ~ ("," ~ (mailbox | cfws))*)+ }
#[tracable_parser]
fn obs_mbox_list(input: Span) -> IResult<Span, Vec<Mailbox>> {
    let (loc, entries) = context(
        "obs_mbox_list",
        many1(preceded(
            many0(preceded(opt(cfws), char(','))),
            tuple((
                mailbox,
                many0(preceded(
                    char(','),
                    alt((map(mailbox, |m| Some(m)), map(cfws, |_| None))),
                )),
            )),
        )),
    )(input)?;

    let mut result: Vec<Mailbox> = vec![];

    for (first, boxes) in entries {
        result.push(first);
        for b in boxes {
            if let Some(m) = b {
                result.push(m);
            }
        }
    }

    Ok((loc, result))
}

// mailbox = { name_addr | addr_spec }
#[tracable_parser]
fn mailbox(input: Span) -> IResult<Span, Mailbox> {
    if let Ok(res) = name_addr(input) {
        Ok(res)
    } else {
        let (loc, aspec) = context("mailbox", addr_spec)(input)?;
        Ok((
            loc,
            Mailbox {
                name: None,
                address: format!("{}@{}", aspec.local_part, aspec.domain), // FIXME
            },
        ))
    }
}

// name_addr = { display_name? ~ angle_addr }
#[tracable_parser]
fn name_addr(input: Span) -> IResult<Span, Mailbox> {
    let (loc, (name, aspec)) = context("name_addr", tuple((opt(display_name), angle_addr)))(input)?;
    Ok((
        loc,
        Mailbox {
            name,
            address: format!("{}@{}", aspec.local_part, aspec.domain), // FIXME
        },
    ))
}

// display_name = { phrase }
#[tracable_parser]
fn display_name(input: Span) -> IResult<Span, String> {
    context("display_name", phrase)(input)
}

// phrase = { (encoded_word | word)+ | obs_phrase }
// obs_phrase = { (encoded_word | word) ~ (encoded_word | word | dot | cfws)* }
#[tracable_parser]
fn phrase(input: Span) -> IResult<Span, String> {
    let (loc, (a, b)): (Span, (String, Vec<Option<String>>)) = context(
        "phrase",
        tuple((
            alt((encoded_word, word)),
            many0(alt((
                map(encoded_word, Option::Some),
                map(word, Option::Some),
                map(char('.'), |dot| Some(dot.to_string())),
                map(cfws, |_| None),
            ))),
        )),
    )(input)?;
    let mut result = vec![];
    result.push(a);
    for item in b {
        if let Some(item) = item {
            result.push(item);
        }
    }
    let result = result.join(" ");
    Ok((loc, result))
}

// angle_addr = { cfws? ~ "<" ~ addr_spec ~ ">" ~ cfws? | obs_angle_addr }
#[tracable_parser]
fn angle_addr(input: Span) -> IResult<Span, AddrSpec> {
    context(
        "angle_addr",
        alt((
            delimited(
                opt(cfws),
                delimited(char('<'), addr_spec, char('>')),
                opt(cfws),
            ),
            obs_angle_addr,
        )),
    )(input)
}

#[tracable_parser]
// obs_angle_addr = { cfws? ~ "<" ~ obs_route ~ addr_spec ~ ">" ~ cfws? }
fn obs_angle_addr(input: Span) -> IResult<Span, AddrSpec> {
    context(
        "obs_angle_addr",
        delimited(
            opt(cfws),
            delimited(char('<'), preceded(obs_route, addr_spec), char('>')),
            opt(cfws),
        ),
    )(input)
}

// obs_route = { obs_domain_list ~ ":" }
// obs_domain_list = { (cfws | ",")* ~ "@" ~ domain ~ ("," ~ cfws? ~ ("@" ~ domain)?)* }
#[tracable_parser]
fn obs_route(input: Span) -> IResult<Span, Span> {
    context(
        "obs_route",
        recognize(terminated(
            tuple((
                many0(alt((cfws, recognize(char(','))))),
                recognize(char('@')),
                recognize(domain),
                many0(tuple((
                    char(','),
                    opt(cfws),
                    opt(tuple((char('@'), domain))),
                ))),
            )),
            char(':'),
        )),
    )(input)
}

// addr_spec = { local_part ~ "@" ~ domain }
#[tracable_parser]
fn addr_spec(input: Span) -> IResult<Span, AddrSpec> {
    let (loc, (local_part, domain)) =
        context("addr_spec", separated_pair(local_part, char('@'), domain))(input)?;
    Ok((loc, AddrSpec { local_part, domain }))
}

fn parse_with<'a, R, F>(text: &'a str, parser: F) -> Result<R>
where
    F: Fn(Span<'a>) -> IResult<Span<'a>, R>,
{
    let input = make_span(text);
    let (_, result) =
        all_consuming(parser)(input).map_err(|err| MailParsingError::from_nom(input, err))?;
    Ok(result)
}

#[cfg(test)]
#[test]
fn test_addr_spec() {
    k9::snapshot!(
        parse_with("darth.vader@a.galaxy.far.far.away", addr_spec),
        r#"
Ok(
    AddrSpec {
        local_part: "darth.vader",
        domain: "a.galaxy.far.far.away",
    },
)
"#
    );

    k9::snapshot!(
        parse_with("\"darth.vader\"@a.galaxy.far.far.away", addr_spec),
        r#"
Ok(
    AddrSpec {
        local_part: "darth.vader",
        domain: "a.galaxy.far.far.away",
    },
)
"#
    );

    k9::snapshot!(
        parse_with("\"darth\".vader@a.galaxy.far.far.away", addr_spec),
        r#"
Err(
    HeaderParse(
        "0: at line 1:
"darth".vader@a.galaxy.far.far.away
       ^
expected '@', found .

1: at line 1, in addr_spec:
"darth".vader@a.galaxy.far.far.away
^

",
    ),
)
"#
    );

    k9::snapshot!(
        parse_with("a@[127.0.0.1]", addr_spec),
        r#"
Ok(
    AddrSpec {
        local_part: "a",
        domain: "[127.0.0.1]",
    },
)
"#
    );

    k9::snapshot!(
        parse_with("a@[IPv6::1]", addr_spec),
        r#"
Ok(
    AddrSpec {
        local_part: "a",
        domain: "[IPv6::1]",
    },
)
"#
    );
}

// atom = { cfws? ~ atext ~ cfws? }
fn atom(input: Span) -> IResult<Span, String> {
    let (loc, text) = context("atom", delimited(opt(cfws), atext, opt(cfws)))(input)?;
    Ok((loc, text.to_string()))
}

// word = { atom | quoted_string }
fn word(input: Span) -> IResult<Span, String> {
    context("word", alt((atom, quoted_string)))(input)
}

// obs_local_part = { word ~ (dot ~ word)* }
#[tracable_parser]
fn obs_local_part(input: Span) -> IResult<Span, String> {
    let (loc, (word, dotted_words)) = context(
        "obs_local_part",
        tuple((word, many0(tuple((char('.'), word))))),
    )(input)?;
    let mut result = String::new();

    result.push_str(&word);
    for (dot, w) in dotted_words {
        result.push(dot);
        result.push_str(&w);
    }

    Ok((loc, result))
}

// local_part = { dot_atom | quoted_string | obs_local_part }
#[tracable_parser]
fn local_part(input: Span) -> IResult<Span, String> {
    context("local_part", alt((dot_atom, quoted_string, obs_local_part)))(input)
}

// domain = { dot_atom | domain_literal | obs_domain }
#[tracable_parser]
fn domain(input: Span) -> IResult<Span, String> {
    context("domain", alt((dot_atom, domain_literal, obs_domain)))(input)
}

// obs_domain = { atom ~ ( dot ~ atom)* }
#[tracable_parser]
fn obs_domain(input: Span) -> IResult<Span, String> {
    let (loc, (atom, dotted_atoms)) =
        context("obs_domain", tuple((atom, many0(tuple((char('.'), atom))))))(input)?;
    let mut result = String::new();

    result.push_str(&atom);
    for (dot, w) in dotted_atoms {
        result.push(dot);
        result.push_str(&w);
    }

    Ok((loc, result))
}

// domain_literal = { cfws? ~ "[" ~ (fws? ~ dtext)* ~ fws? ~ "]" ~ cfws? }
#[tracable_parser]
fn domain_literal(input: Span) -> IResult<Span, String> {
    let (loc, (bits, trailer)) = context(
        "domain_literal",
        delimited(
            opt(cfws),
            delimited(
                char('['),
                tuple((
                    many0(tuple((opt(fws), alt((satisfy(is_dtext), quoted_pair))))),
                    opt(fws),
                )),
                char(']'),
            ),
            opt(cfws),
        ),
    )(input)?;

    let mut result = String::new();
    result.push('[');
    for (a, b) in bits {
        if let Some(a) = a {
            result.push_str(&a);
        }
        result.push(b);
    }
    if let Some(t) = trailer {
        result.push_str(&t);
    }
    result.push(']');
    Ok((loc, result))
}

// dot_atom_text = @{ atext ~ ("." ~ atext)* }
// dot_atom = { cfws? ~ dot_atom_text ~ cfws? }
#[tracable_parser]
fn dot_atom(input: Span) -> IResult<Span, String> {
    let (loc, (a, b)) = context(
        "dot_atom",
        delimited(
            opt(cfws),
            tuple((atext, many0(preceded(char('.'), atext)))),
            opt(cfws),
        ),
    )(input)?;

    let mut result = String::new();
    result.push_str(&a);
    for item in b {
        result.push('.');
        result.push_str(&item);
    }

    Ok((loc, result))
}

#[cfg(test)]
#[test]
fn test_dot_atom() {
    k9::snapshot!(
        parse_with("hello", dot_atom),
        r#"
Ok(
    "hello",
)
"#
    );

    k9::snapshot!(
        parse_with("hello.there", dot_atom),
        r#"
Ok(
    "hello.there",
)
"#
    );

    k9::snapshot!(
        parse_with("hello.", dot_atom),
        r#"
Err(
    HeaderParse(
        "0: at line 1, in Eof:
hello.
     ^

",
    ),
)
"#
    );

    k9::snapshot!(
        parse_with("(wat)hello", dot_atom),
        r#"
Ok(
    "hello",
)
"#
    );
}

// cfws = { ( (fws? ~ comment)+ ~ fws?) | fws }
#[tracable_parser]
fn cfws(input: Span) -> IResult<Span, Span> {
    context(
        "cfws",
        recognize(alt((
            recognize(tuple((many1(tuple((opt(fws), comment))), opt(fws)))),
            fws,
        ))),
    )(input)
}

// comment = { "(" ~ (fws? ~ ccontent)* ~ fws? ~ ")" }
#[tracable_parser]
fn comment(input: Span) -> IResult<Span, Span> {
    context(
        "comment",
        recognize(tuple((
            char('('),
            many0(tuple((opt(fws), ccontent))),
            opt(fws),
            char(')'),
        ))),
    )(input)
}

#[cfg(test)]
#[test]
fn test_comment() {
    k9::snapshot!(
        parse_with("(wat)", comment),
        r#"
Ok(
    LocatedSpan {
        offset: 0,
        line: 1,
        fragment: "(wat)",
        extra: TracableInfo,
    },
)
"#
    );
}

// ccontent = { ctext | quoted_pair | comment | encoded_word }
#[tracable_parser]
fn ccontent(input: Span) -> IResult<Span, Span> {
    context(
        "ccontent",
        recognize(alt((
            recognize(satisfy(is_ctext)),
            recognize(quoted_pair),
            comment,
            recognize(encoded_word),
        ))),
    )(input)
}

// quoted_pair = { ( "\\"  ~ (vchar | wsp)) | obs_qp }
// obs_qp = { "\\" ~ ( "\u{00}" | obs_no_ws_ctl | "\r" | "\n") }
#[tracable_parser]
fn quoted_pair(input: Span) -> IResult<Span, char> {
    context(
        "quoted_pair",
        preceded(
            char('\\'),
            satisfy(|c| match c {
                '\u{00}' | '\r' | '\n' | ' ' => true,
                c => is_obs_no_ws_ctl(c) || is_vchar(c),
            }),
        ),
    )(input)
}

// encoded_word = { "=?" ~ charset ~ ("*" ~ language)? ~ "?" ~ encoding ~ "?" ~ encoded_text ~ "?=" }
#[tracable_parser]
fn encoded_word(input: Span) -> IResult<Span, String> {
    let (loc, (charset, _language, _, encoding, _, text)) = context(
        "encoded_word",
        delimited(
            tag("=?"),
            tuple((
                charset,
                opt(preceded(char('*'), language)),
                char('?'),
                encoding,
                char('?'),
                encoded_text,
            )),
            tag("?="),
        ),
    )(input)?;

    let bytes = match *encoding.fragment() {
        "B" | "b" => data_encoding::BASE64_MIME
            .decode(text.as_bytes())
            .map_err(|_err| make_context_error(input, "encoded_word: base64 decode failed"))?,
        "Q" | "q" => {
            quoted_printable::decode(text.replace("_", " "), quoted_printable::ParseMode::Robust)
                .map_err(|_err| {
                    make_context_error(input, "encoded_word: quoted printable decode failed")
                })?
        }
        _ => {
            return Err(make_context_error(
                input,
                "encoded_word: invalid encoding, expected one of b, B, q or Q",
            ));
        }
    };

    let charset = Charset::for_label_no_replacement(charset.as_bytes())
        .ok_or_else(|| make_context_error(input, "encoded_word: unsupported charset"))?;

    let (decoded, _malformed) = charset.decode_without_bom_handling(&bytes);

    Ok((loc, decoded.to_string()))
}

// token = { !(" " | especials | ctl) ~ char }
#[tracable_parser]
fn token(input: Span) -> IResult<Span, char> {
    context("token", satisfy(is_token))(input)
}

// charset = @{ (!"*" ~ token)+ }
#[tracable_parser]
fn charset(input: Span) -> IResult<Span, Span> {
    context("charset", take_while1(|c| c != '*' && is_token(c)))(input)
}

// language = @{ token+ }
#[tracable_parser]
fn language(input: Span) -> IResult<Span, Span> {
    context("language", take_while1(|c| c != '*' && is_token(c)))(input)
}

// encoding = @{ token+ }
#[tracable_parser]
fn encoding(input: Span) -> IResult<Span, Span> {
    context("encoding", take_while1(|c| c != '*' && is_token(c)))(input)
}

// encoded_text = @{ (!( " " | "?") ~ vchar)+ }
#[tracable_parser]
fn encoded_text(input: Span) -> IResult<Span, Span> {
    context(
        "encoded_text",
        take_while1(|c| is_vchar(c) && c != ' ' && c != '?'),
    )(input)
}

// quoted_string = { cfws? ~ "\"" ~ (fws? ~ qcontent)* ~ fws? ~ "\"" ~ cfws? }
#[tracable_parser]
fn quoted_string(input: Span) -> IResult<Span, String> {
    let (loc, (bits, trailer)) = context(
        "quoted_string",
        delimited(
            opt(cfws),
            delimited(
                char('"'),
                tuple((many0(tuple((opt(fws), qcontent))), opt(fws))),
                char('"'),
            ),
            opt(cfws),
        ),
    )(input)?;

    let mut result = String::new();
    for (a, b) in bits {
        if let Some(a) = a {
            result.push_str(&a);
        }
        result.push(b);
    }
    if let Some(t) = trailer {
        result.push_str(&t);
    }
    Ok((loc, result))
}

// qcontent = { qtext | quoted_pair }
#[tracable_parser]
fn qcontent(input: Span) -> IResult<Span, char> {
    context("qcontent", alt((satisfy(is_qtext), quoted_pair)))(input)
}

#[derive(Parser)]
#[grammar = "rfc5322.pest"]
pub struct Parser;

impl Parser {
    pub fn parse_mailbox_list_header(text: &str) -> Result<MailboxList> {
        parse_with(text, mailbox_list)
    }

    fn parse_mailbox_list(pairs: Pairs<Rule>) -> Result<MailboxList> {
        let mut result: Vec<Mailbox> = vec![];

        for p in pairs {
            result.push(Self::parse_mailbox(p.into_inner())?);
        }

        Ok(MailboxList(result))
    }

    pub fn parse_mailbox_header(text: &str) -> Result<Mailbox> {
        parse_with(text, mailbox)
    }

    pub fn parse_address_list_header(text: &str) -> Result<AddressList> {
        let mut pairs = Self::parse(Rule::parse_address_list, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        let mut result: Vec<Address> = vec![];

        while let Some(pair) = pairs.next() {
            result.push(Self::parse_address(pair.into_inner())?);
        }

        Ok(AddressList(result))
    }

    pub fn parse_msg_id_header(text: &str) -> Result<String> {
        let pairs = Self::parse(Rule::parse_msg_id, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        Self::parse_msg_id(pairs)
    }

    pub fn parse_msg_id_header_list(text: &str) -> Result<Vec<String>> {
        let pairs = Self::parse(Rule::parse_msg_id_list, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        let mut result = vec![];
        for p in pairs {
            result.push(Self::parse_msg_id(p.into_inner())?);
        }
        Ok(result)
    }

    fn parse_msg_id(pairs: Pairs<Rule>) -> Result<String> {
        let mut result = String::new();
        for p in pairs {
            match p.as_rule() {
                Rule::id_left => {
                    let content = p.into_inner().next().unwrap();
                    match content.as_rule() {
                        Rule::dot_atom_text => {
                            result += content.as_str();
                        }
                        Rule::obs_id_left => {
                            result +=
                                &Self::parse_local_part(content.into_inner().next().unwrap())?;
                        }
                        rule => {
                            return Err(MailParsingError::HeaderParse(format!(
                                "Invalid {rule:?} {content:#?} in parse_msg_id id_left"
                            )))
                        }
                    }
                }
                Rule::id_right => {
                    let content = p.into_inner().next().unwrap();
                    match content.as_rule() {
                        Rule::dot_atom_text => {
                            result.push('@');
                            result += content.as_str();
                            return Ok(result);
                        }
                        Rule::no_fold_literal => {
                            result.push('@');
                            result += &Self::parse_domain_literal(content)?;
                            return Ok(result);
                        }
                        Rule::obs_id_right => {
                            result.push('@');
                            result += &Self::parse_domain(content.into_inner().next().unwrap())?;
                            return Ok(result);
                        }
                        rule => {
                            return Err(MailParsingError::HeaderParse(format!(
                                "Invalid {rule:?} {content:#?} in parse_msg_id id_left"
                            )))
                        }
                    }
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Invalid {rule:?} {p:#?} in parse_msg_id"
                    )))
                }
            }
        }
        Err(MailParsingError::HeaderParse(format!(
            "Unreachable end of loop in parse_msg_id"
        )))
    }

    pub fn parse_content_type_header(text: &str) -> Result<MimeParameters> {
        let pairs = Self::parse(Rule::parse_content_type, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        Self::parse_content_type(pairs)
    }

    fn parse_content_type(pairs: Pairs<Rule>) -> Result<MimeParameters> {
        let mut value = String::new();
        let mut parameters = vec![];

        for p in pairs {
            match p.as_rule() {
                Rule::mime_type => {
                    value.push_str(p.as_str());
                    value.push('/');
                }
                Rule::subtype => {
                    value.push_str(p.as_str());
                }
                Rule::cfws => {}
                Rule::parameter => {
                    parameters.push(Self::parse_parameter(p.into_inner())?);
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_content_type"
                    )))
                }
            }
        }

        Ok(MimeParameters { value, parameters })
    }

    fn parse_parameter(pairs: Pairs<Rule>) -> Result<(String, String)> {
        for p in pairs {
            match p.as_rule() {
                Rule::regular_parameter => return Self::parse_regular_parameter(p.into_inner()),
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_parameter"
                    )))
                }
            };
        }
        todo!();
    }

    fn parse_regular_parameter(pairs: Pairs<Rule>) -> Result<(String, String)> {
        let mut name = String::new();

        for p in pairs {
            match p.as_rule() {
                Rule::attribute => {
                    name = p.as_str().to_string();
                }
                Rule::value => {
                    let value = Self::parse_value(p.into_inner())?;
                    return Ok((name, value));
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_regular_parameter"
                    )))
                }
            };
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_regular_parameter".to_string(),
        ))
    }

    fn parse_value(pairs: Pairs<Rule>) -> Result<String> {
        for p in pairs {
            match p.as_rule() {
                Rule::mime_token => {
                    return Ok(p.as_str().to_string());
                }
                Rule::quoted_string => {
                    return Self::parse_quoted_string(p);
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_value"
                    )))
                }
            };
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_value".to_string(),
        ))
    }

    pub fn parse_unstructured_header(text: &str) -> Result<String> {
        let mut pairs = Self::parse(Rule::parse_unstructured, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        Self::parse_unstructured(pairs.next().unwrap().into_inner())
    }

    fn parse_unstructured(pairs: Pairs<Rule>) -> Result<String> {
        #[derive(Debug)]
        enum Word {
            Encoded(String),
            Text(String),
            Fws,
        }

        let mut words: Vec<Word> = vec![];

        for p in pairs {
            match p.as_rule() {
                Rule::encoded_word => {
                    // Fws between encoded words is elided
                    if words.len() >= 2
                        && matches!(words.last(), Some(Word::Fws))
                        && matches!(words[words.len() - 2], Word::Encoded(_))
                    {
                        words.pop();
                    }
                    words.push(Word::Encoded(Self::parse_encoded_word(p)?));
                }
                Rule::fws | Rule::cfws => {
                    words.push(Word::Fws);
                }
                Rule::obs_utext => match words.last_mut() {
                    Some(Word::Text(prior)) => prior.push_str(p.as_str()),
                    _ => words.push(Word::Text(p.as_str().to_string())),
                },
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_unstructured"
                    )))
                }
            };
        }

        let mut result = String::new();
        for word in &words {
            match word {
                Word::Encoded(s) | Word::Text(s) => {
                    result += s;
                }
                Word::Fws => {
                    result.push(' ');
                }
            }
        }
        Ok(result)
    }

    fn parse_address(pairs: Pairs<Rule>) -> Result<Address> {
        for p in pairs {
            match p.as_rule() {
                Rule::mailbox => return Ok(Address::Mailbox(Self::parse_mailbox(p.into_inner())?)),
                Rule::group => return Self::parse_group(p.into_inner()),
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Expected mailbox or group, but had {rule:?} {p:?}"
                    )))
                }
            };
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_address".to_string(),
        ))
    }

    fn parse_group(pairs: Pairs<Rule>) -> Result<Address> {
        let mut name = String::new();

        for p in pairs {
            match p.as_rule() {
                Rule::display_name => {
                    name = Self::parse_display_name(p)?;
                }
                Rule::cfws => {}
                Rule::group_list => {
                    for p in p.into_inner() {
                        match p.as_rule() {
                            Rule::mailbox_list => {
                                return Ok(Address::Group {
                                    name,
                                    entries: Self::parse_mailbox_list(p.into_inner())?,
                                });
                            }
                            Rule::obs_group_list | Rule::cfws => {}
                            rule => {
                                return Err(MailParsingError::HeaderParse(format!(
                                    "Unexpected {rule:?} {p:?} in parse_group group_list"
                                )))
                            }
                        }
                    }
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:?} in parse_group"
                    )))
                }
            };
        }

        Ok(Address::Group {
            name,
            entries: MailboxList(vec![]),
        })
    }

    fn parse_mailbox(pairs: Pairs<Rule>) -> Result<Mailbox> {
        for p in pairs {
            match p.as_rule() {
                Rule::name_addr => return Self::parse_name_addr(p),
                Rule::addr_spec => {
                    return Ok(Mailbox {
                        name: None,
                        address: Self::parse_addr_spec(p)?,
                    })
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Expected name_addr or addr_spec, but had {rule:?} {p:?}"
                    )))
                }
            };
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_mailbox".to_string(),
        ))
    }

    fn parse_dot_atom(pair: Pair<Rule>) -> Result<String> {
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::cfws => {}
                Rule::dot_atom_text => return Ok(p.as_str().to_string()),
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "invalid {rule:?} {p:#?} in parse_dot_atom"
                    )))
                }
            }
        }

        Err(MailParsingError::HeaderParse(format!(
            "Unreachable end of loop in parse_dot_atom"
        )))
    }

    fn parse_local_part(pair: Pair<Rule>) -> Result<String> {
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::dot_atom => return Self::parse_dot_atom(p),
                Rule::quoted_string => return Self::parse_quoted_string(p),
                Rule::obs_local_part => return Self::parse_obs_local_part(p),
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Invalid {rule:?} {p:#?} in parse_local_part"
                    )))
                }
            }
        }
        Err(MailParsingError::HeaderParse(format!(
            "Unreachable end of loop in parse_local_part"
        )))
    }

    fn parse_obs_local_part(pair: Pair<Rule>) -> Result<String> {
        let mut result = String::new();
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::word => {
                    result += &Self::parse_word(p)?;
                }
                Rule::dot => {
                    result += p.as_str();
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Invalid {rule:?} {p:#?} in parse_obs_local_part"
                    )))
                }
            }
        }
        Ok(result)
    }

    fn parse_obs_domain(pair: Pair<Rule>) -> Result<String> {
        let mut result = String::new();
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::atom => {
                    result += &Self::parse_atom(p)?;
                }
                Rule::dot => {
                    result += p.as_str();
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Invalid {rule:?} {p:#?} in parse_obs_domain"
                    )))
                }
            }
        }
        Ok(result)
    }

    fn parse_domain(pair: Pair<Rule>) -> Result<String> {
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::dot_atom => return Self::parse_dot_atom(p),
                Rule::domain_literal => return Self::parse_domain_literal(p),
                Rule::obs_domain => return Self::parse_obs_domain(p),
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_domain"
                    )))
                }
            }
        }

        Err(MailParsingError::HeaderParse(format!(
            "Unreachable end of loop in parse_domain"
        )))
    }

    fn parse_domain_literal(pair: Pair<Rule>) -> Result<String> {
        let mut result = "[".to_string();
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::fws | Rule::cfws => {}
                Rule::dtext => {
                    let dtext = p.as_str();
                    if dtext.len() == 2 {
                        // Must be quoted_pair
                        result.push_str(&dtext[1..]);
                    } else {
                        result.push_str(dtext);
                    }
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_domain_literal"
                    )))
                }
            }
        }

        result.push(']');

        Ok(result)
    }

    fn parse_addr_spec(pair: Pair<Rule>) -> Result<String> {
        let mut result = String::new();

        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::local_part => {
                    result = Self::parse_local_part(p)?;
                    result.push('@');
                }
                Rule::domain => {
                    result += &Self::parse_domain(p)?;
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Invalid {rule:?} {p:#?} in parse_addr_spec"
                    )))
                }
            }
        }

        Ok(result)
    }

    fn parse_angle_addr(pair: Pair<Rule>) -> Result<String> {
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::addr_spec => return Self::parse_addr_spec(p),
                Rule::cfws => {}
                Rule::obs_angle_addr => return Self::parse_obs_angle_addr(p),
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_angle_addr"
                    )))
                }
            }
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_angle_addr".to_string(),
        ))
    }

    fn parse_obs_angle_addr(pair: Pair<Rule>) -> Result<String> {
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::addr_spec => return Self::parse_addr_spec(p),
                Rule::cfws => {}
                Rule::obs_route => {
                    // We simply ignore this, as the RFC recommends
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_obs_angle_addr"
                    )))
                }
            }
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_obs_angle_addr".to_string(),
        ))
    }

    fn parse_word(pair: Pair<Rule>) -> Result<String> {
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::atom => return Self::parse_atom(p),
                Rule::quoted_string => return Self::parse_quoted_string(p),
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Invalid {rule:?} {p:#?} in parse_word"
                    )))
                }
            }
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_word".to_string(),
        ))
    }

    fn parse_quoted_string(pair: Pair<Rule>) -> Result<String> {
        let mut result = String::new();
        let mut fws = false;

        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::fws | Rule::cfws => {
                    if !result.is_empty() && !fws {
                        result.push(' ');
                    }
                    fws = true;
                }
                Rule::qcontent => {
                    fws = false;
                    let content = p.into_inner().next().unwrap();
                    match content.as_rule() {
                        Rule::qtext => result.push_str(content.as_str()),
                        Rule::quoted_pair => result.push_str(&content.as_str()[1..]),
                        rule => {
                            return Err(MailParsingError::HeaderParse(format!(
                                "Invalid {rule:?} {content:#?} in parse_quoted_string qcontent"
                            )))
                        }
                    }
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Invalid {rule:?} {p:#?} in parse_quoted_string"
                    )))
                }
            }
        }

        if fws {
            result.pop();
        }

        Ok(result)
    }

    // We parse `language` for completeness, but we do not use it
    #[allow(unused_assignments, unused_variables)]
    fn parse_encoded_word(pair: Pair<Rule>) -> Result<String> {
        let mut charset = String::new();
        let mut language = String::new();
        let mut encoding = String::new();
        let mut text = String::new();

        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::charset => {
                    charset = p.as_str().to_string();
                }
                Rule::encoding => {
                    encoding = p.as_str().to_string();
                }
                Rule::language => {
                    language = p.as_str().to_string();
                }
                Rule::encoded_text => {
                    text = p.as_str().to_string();
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Invalid {rule:?} {p:#?} in parse_encoded_word"
                    )))
                }
            }
        }

        let bytes = match encoding.as_str() {
            "B" | "b" => data_encoding::BASE64_MIME
                .decode(text.as_bytes())
                .map_err(|err| {
                    MailParsingError::HeaderParse(format!(
                        "Invalid base64 encoding: {err:#} {text:?}"
                    ))
                })?,
            "Q" | "q" => quoted_printable::decode(
                text.replace("_", " "),
                quoted_printable::ParseMode::Robust,
            )
            .map_err(|err| {
                MailParsingError::HeaderParse(format!(
                    "Invalid quoted printable encoding: {err:#} {text:?}"
                ))
            })?,
            _ => {
                return Err(MailParsingError::HeaderParse(format!(
                    "Invalid encoding {encoding} in parse_encoded_word"
                )))
            }
        };

        let charset = Charset::for_label_no_replacement(charset.as_bytes()).ok_or_else(|| {
            MailParsingError::HeaderParse(format!("unsupported charset: {charset:?}"))
        })?;

        let (decoded, _malformed) = charset.decode_without_bom_handling(&bytes);

        Ok(decoded.into())
    }

    fn parse_atom(pair: Pair<Rule>) -> Result<String> {
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::atext => return Ok(p.as_str().to_string()),
                Rule::cfws => {}
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} {p:#?} in parse_atom"
                    )))
                }
            }
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_atom".to_string(),
        ))
    }

    fn parse_phrase(pair: Pair<Rule>) -> Result<Vec<String>> {
        let mut words = vec![];
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::word => words.push(Self::parse_word(p)?),
                Rule::encoded_word => words.push(Self::parse_encoded_word(p)?),
                Rule::obs_phrase => {
                    words.append(&mut Self::parse_obs_phrase(p)?);
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} in parse_phrase"
                    )))
                }
            }
        }
        Ok(words)
    }

    fn parse_obs_phrase(pair: Pair<Rule>) -> Result<Vec<String>> {
        let mut words = vec![];
        let mut current_word = String::new();
        for p in pair.into_inner() {
            match p.as_rule() {
                Rule::word => {
                    current_word += &Self::parse_word(p)?;
                }
                Rule::encoded_word => {
                    current_word += &Self::parse_encoded_word(p)?;
                }
                Rule::dot => {
                    current_word.push('.');
                }
                Rule::cfws => {
                    if !current_word.is_empty() {
                        words.push(current_word.clone());
                        current_word.clear();
                    }
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unexpected {rule:?} in parse_obs_phrase"
                    )))
                }
            }
        }
        Ok(words)
    }

    fn parse_display_name(pair: Pair<Rule>) -> Result<String> {
        let words = Self::parse_phrase(pair.into_inner().next().unwrap())?;
        Ok(words.join(" "))
    }

    fn parse_name_addr(name_addr: Pair<Rule>) -> Result<Mailbox> {
        let name_addr = name_addr.into_inner();
        let mut name = None;

        for p in name_addr {
            match p.as_rule() {
                Rule::display_name => {
                    name.replace(Self::parse_display_name(p)?);
                    //name.replace(Self::text_ignoring_cfws(p, true)?);
                }
                Rule::angle_addr => {
                    let address = Self::parse_angle_addr(p)?;
                    return Ok(Mailbox { name, address });
                }
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "parse_name_addr: invalid {rule:?} for {p:?}"
                    )))
                }
            };
        }

        Err(MailParsingError::HeaderParse(format!(
            "Unreachable end of loop in parse_name_addr"
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddrSpec {
    pub local_part: String,
    pub domain: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    Mailbox(Mailbox),
    Group { name: String, entries: MailboxList },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressList(Vec<Address>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxList(Vec<Mailbox>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    pub name: Option<String>,
    pub address: String, // FIXME: AddrSpec
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MimeParameters {
    pub value: String,
    parameters: Vec<(String, String)>,
}

impl MimeParameters {
    pub fn get(&self, name: &str) -> Option<&str> {
        for entry in &self.parameters {
            if entry.0.eq_ignore_ascii_case(name) {
                return Some(&entry.1);
            }
        }
        None
    }
}

pub(crate) fn qp_encode(s: &str) -> String {
    let prefix = b"=?UTF-8?q?";
    let suffix = b"?=";
    let limit = 74 - (prefix.len() + suffix.len());

    static HEX_CHARS: &[u8] = &[
        b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'A', b'B', b'C', b'D', b'E',
        b'F',
    ];

    let mut result = Vec::with_capacity(s.len());

    result.extend_from_slice(prefix);
    let mut line_length = 0;

    enum Byte {
        Passthru(u8),
        Encode(u8),
    }

    for c in s.bytes() {
        let b = if (c.is_ascii_alphanumeric() || c.is_ascii_punctuation())
            && c != b'?'
            && c != b'='
            && c != b' '
            && c != b'\t'
        {
            Byte::Passthru(c)
        } else if c == b' ' {
            Byte::Passthru(b'_')
        } else {
            Byte::Encode(c)
        };

        let need_len = match b {
            Byte::Passthru(_) => 1,
            Byte::Encode(_) => 3,
        };

        if need_len > limit - line_length {
            // Need to wrap
            result.extend_from_slice(suffix);
            result.extend_from_slice(b"\r\n\t");
            result.extend_from_slice(prefix);
            line_length = 0;
        }

        match b {
            Byte::Passthru(c) => {
                result.push(c);
            }
            Byte::Encode(c) => {
                result.push(b'=');
                result.push(HEX_CHARS[(c as usize) >> 4]);
                result.push(HEX_CHARS[(c as usize) & 0x0f]);
            }
        }

        line_length += need_len;
    }

    if line_length > 0 {
        result.extend_from_slice(suffix);
    }

    // Safety: we ensured that everything we output is in the ASCII
    // range, therefore the string is valid UTF-8
    unsafe { String::from_utf8_unchecked(result) }
}

#[cfg(test)]
#[test]
fn test_qp_encode() {
    let encoded = qp_encode(
        "hello, I am a line that is this long, or maybe a little \
        bit longer than this, and that should get wrapped by the encoder",
    );
    k9::snapshot!(
        encoded,
        r#"
=?UTF-8?q?hello,_I_am_a_line_that_is_this_long,_or_maybe_a_little_bit_lo?=\r
\t=?UTF-8?q?nger_than_this,_and_that_should_get_wrapped_by_the_encoder?=
"#
    );
}

/// Quote input string `s`, using a backslash escape,
/// any of the characters listed in needs_quote
pub(crate) fn quote_string(s: &str, needs_quote: &str) -> String {
    if s.chars().any(|c| needs_quote.contains(c)) {
        let mut result = String::with_capacity(s.len() + 4);
        result.push('"');
        for c in s.chars() {
            if needs_quote.contains(c) {
                result.push('\\');
            }
            result.push(c);
        }
        result.push('"');
        result
    } else {
        s.to_string()
    }
}

#[cfg(test)]
#[test]
fn test_quote_string() {
    let nq = "\\\"";
    k9::snapshot!(quote_string("hello", nq), "hello");
    k9::snapshot!(quote_string("hello there", nq), "hello there");
    k9::snapshot!(
        quote_string("hello \"there\"", nq),
        r#""hello \\"there\\"""#
    );
    k9::snapshot!(
        quote_string("hello c:\\backslash", nq),
        r#""hello c:\\\\backslash""#
    );
}

impl EncodeHeaderValue for Mailbox {
    fn encode_value(&self) -> SharedString<'static> {
        match &self.name {
            Some(name) => {
                let mut value = if name.is_ascii() {
                    quote_string(name, "\\\"")
                } else {
                    qp_encode(name)
                };

                value.push_str(" <");
                value.push_str(&self.address);
                value.push('>');
                value.into()
            }
            None => format!("<{}>", self.address).into(),
        }
    }
}

impl EncodeHeaderValue for MailboxList {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = String::new();
        for mailbox in &self.0 {
            if !result.is_empty() {
                result.push_str(",\r\n\t");
            }
            result.push_str(&mailbox.encode_value());
        }
        result.into()
    }
}

impl EncodeHeaderValue for Address {
    fn encode_value(&self) -> SharedString<'static> {
        match self {
            Self::Mailbox(mbox) => mbox.encode_value(),
            Self::Group { name, entries } => {
                let mut result = format!("{name}:");
                result += &entries.encode_value();
                result.push(';');
                result.into()
            }
        }
    }
}

impl EncodeHeaderValue for AddressList {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = String::new();
        for address in &self.0 {
            if !result.is_empty() {
                result.push_str(",\r\n\t");
            }
            result.push_str(&address.encode_value());
        }
        result.into()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Header, MimePart};

    #[test]
    fn mailbox_list_singular() {
        let message = concat!(
            "From:  Someone (hello) <someone@example.com>, other@example.com,\n",
            "  \"John \\\"Smith\\\"\" (comment) \"More Quotes\" (more comment) <someone(another comment)@crazy.example.com(woot)>\n",
            "\n",
            "I am the body"
        );
        let msg = MimePart::parse(message).unwrap();
        let list = match msg.headers().from() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };

        k9::snapshot!(
            list,
            r#"
Some(
    MailboxList(
        [
            Mailbox {
                name: Some(
                    "Someone",
                ),
                address: "someone@example.com",
            },
            Mailbox {
                name: None,
                address: "other@example.com",
            },
            Mailbox {
                name: Some(
                    "John "Smith" More Quotes",
                ),
                address: "someone@crazy.example.com",
            },
        ],
    ),
)
"#
        );
    }

    #[test]
    fn sender() {
        let message = "Sender: someone@[127.0.0.1]\n\n\n";
        let msg = MimePart::parse(message).unwrap();
        let list = match msg.headers().sender() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    Mailbox {
        name: None,
        address: "someone@[127.0.0.1]",
    },
)
"#
        );
    }

    #[test]
    fn domain_literal() {
        let message = "From: someone@[127.0.0.1]\n\n\n";
        let msg = MimePart::parse(message).unwrap();
        let list = match msg.headers().from() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    MailboxList(
        [
            Mailbox {
                name: None,
                address: "someone@[127.0.0.1]",
            },
        ],
    ),
)
"#
        );
    }

    #[test]
    fn rfc6532() {
        let message = concat!(
            "From: Keith Moore <moore@cs.utk.edu>\n",
            "To: Keld Jørn Simonsen <keld@dkuug.dk>\n",
            "CC: André Pirard <PIRARD@vm1.ulg.ac.be>\n",
            "Subject: Hello André\n",
            "\n\n"
        );
        let msg = MimePart::parse(message).unwrap();
        let list = match msg.headers().from() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    MailboxList(
        [
            Mailbox {
                name: Some(
                    "Keith Moore",
                ),
                address: "moore@cs.utk.edu",
            },
        ],
    ),
)
"#
        );

        let list = match msg.headers().to() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: Some(
                        "Keld Jørn Simonsen",
                    ),
                    address: "keld@dkuug.dk",
                },
            ),
        ],
    ),
)
"#
        );

        let list = match msg.headers().cc() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: Some(
                        "André Pirard",
                    ),
                    address: "PIRARD@vm1.ulg.ac.be",
                },
            ),
        ],
    ),
)
"#
        );
        let list = match msg.headers().subject() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    "Hello André",
)
"#
        );
    }

    #[test]
    fn rfc2047() {
        let message = concat!(
            "From: =?US-ASCII?Q?Keith_Moore?= <moore@cs.utk.edu>\n",
            "To: =?ISO-8859-1*en-us?Q?Keld_J=F8rn_Simonsen?= <keld@dkuug.dk>\n",
            "CC: =?ISO-8859-1?Q?Andr=E9?= Pirard <PIRARD@vm1.ulg.ac.be>\n",
            "Subject: Hello =?ISO-8859-1?B?SWYgeW91IGNhbiByZWFkIHRoaXMgeW8=?=\n",
            "  =?ISO-8859-2?B?dSB1bmRlcnN0YW5kIHRoZSBleGFtcGxlLg==?=\n",
            "\n\n"
        );
        let msg = MimePart::parse(message).unwrap();
        let list = match msg.headers().from() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    MailboxList(
        [
            Mailbox {
                name: Some(
                    "Keith Moore",
                ),
                address: "moore@cs.utk.edu",
            },
        ],
    ),
)
"#
        );

        let list = match msg.headers().to() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: Some(
                        "Keld Jørn Simonsen",
                    ),
                    address: "keld@dkuug.dk",
                },
            ),
        ],
    ),
)
"#
        );

        let list = match msg.headers().cc() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: Some(
                        "André Pirard",
                    ),
                    address: "PIRARD@vm1.ulg.ac.be",
                },
            ),
        ],
    ),
)
"#
        );
        let list = match msg.headers().subject() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    "Hello If you can read this you understand the example.",
)
"#
        );
    }

    #[test]
    fn group_addresses() {
        let message = concat!(
            "To: A Group:Ed Jones <c@a.test>,joe@where.test,John <jdoe@one.test>;\n",
            "Cc: Undisclosed recipients:;\n",
            "\n\n\n"
        );
        let msg = MimePart::parse(message).unwrap();
        let list = match msg.headers().to() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list.unwrap(),
        };

        k9::snapshot!(
            list.encode_value(),
            r#"
A Group:Ed Jones <c@a.test>,\r
\t<joe@where.test>,\r
\tJohn <jdoe@one.test>;
"#
        );

        let round_trip = Header::new("To", list.clone());
        k9::assert_equal!(list, round_trip.as_address_list().unwrap());

        k9::snapshot!(
            list,
            r#"
AddressList(
    [
        Group {
            name: "A Group",
            entries: MailboxList(
                [
                    Mailbox {
                        name: Some(
                            "Ed Jones",
                        ),
                        address: "c@a.test",
                    },
                    Mailbox {
                        name: None,
                        address: "joe@where.test",
                    },
                    Mailbox {
                        name: Some(
                            "John",
                        ),
                        address: "jdoe@one.test",
                    },
                ],
            ),
        },
    ],
)
"#
        );

        let list = match msg.headers().cc() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    AddressList(
        [
            Group {
                name: "Undisclosed recipients",
                entries: MailboxList(
                    [],
                ),
            },
        ],
    ),
)
"#
        );
    }

    #[test]
    fn message_id() {
        let message = concat!(
            "Message-Id: <foo@example.com>\n",
            "References: <a@example.com> <b@example.com>\n",
            "  <\"legacy\"@example.com>\n",
            "  <literal@[127.0.0.1]>\n",
            "\n\n\n"
        );
        let msg = MimePart::parse(message).unwrap();
        let list = match msg.headers().message_id() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    "foo@example.com",
)
"#
        );

        let list = match msg.headers().references() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(list) => list,
        };
        k9::snapshot!(
            list,
            r#"
Some(
    [
        "a@example.com",
        "b@example.com",
        "legacy@example.com",
        "literal@[127.0.0.1]",
    ],
)
"#
        );
    }

    #[test]
    fn content_type() {
        let message = "Content-Type: text/plain\n\n\n\n";
        let msg = MimePart::parse(message).unwrap();
        let params = match msg.headers().content_type() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(params) => params,
        };
        k9::snapshot!(
            params,
            r#"
Some(
    MimeParameters {
        value: "text/plain",
        parameters: [],
    },
)
"#
        );

        let message = "Content-Type: text/plain; charset=us-ascii\n\n\n\n";
        let msg = MimePart::parse(message).unwrap();
        let params = match msg.headers().content_type() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(params) => params,
        };
        k9::snapshot!(
            params,
            r#"
Some(
    MimeParameters {
        value: "text/plain",
        parameters: [
            (
                "charset",
                "us-ascii",
            ),
        ],
    },
)
"#
        );

        let message = "Content-Type: text/plain; charset=\"us-ascii\"\n\n\n\n";
        let msg = MimePart::parse(message).unwrap();
        let params = match msg.headers().content_type() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(params) => params,
        };
        k9::snapshot!(
            params,
            r#"
Some(
    MimeParameters {
        value: "text/plain",
        parameters: [
            (
                "charset",
                "us-ascii",
            ),
        ],
    },
)
"#
        );
    }

    /*
    #[test]
    fn content_type_rfc2231() {
        let message = concat!(
            "Content-Type: application/x-stuff;\n",
            "\ttitle*0*=us-ascii'en'This%20is%20even%20more%20\n",
            "\ttitle*1*=%2A%2A%2Afun%2A%2A%2A%20\n",
            "\ttitle*2=\"isn't it!\"\n",
            "\n\n\n"
        );
        let msg = MimePart::parse(message).unwrap();
        let params = match msg.headers().content_type() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(params) => params,
        };
        k9::snapshot!(
            params);
    }
    */
}
