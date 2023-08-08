use crate::headermap::EncodeHeaderValue;
use crate::{MailParsingError, Result, SharedString};
use charset::Charset;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while, take_while1};
use nom::character::complete::{char, satisfy};
use nom::combinator::{all_consuming, map, opt, recognize};
use nom::error::{context, ContextError, VerboseError};
use nom::multi::{many0, many1, separated_list1};
use nom::sequence::{delimited, preceded, separated_pair, terminated, tuple};
use nom_locate::LocatedSpan;
use nom_tracable::{tracable_parser, TracableInfo};

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
                    // Remap \t in particular, because it can render as multiple
                    // columns and defeat the column number calculation provided
                    // by the Span type
                    let line: String = line
                        .chars()
                        .map(|c| match c {
                            '\t' => '\u{2409}',
                            '\r' => '\u{240d}',
                            '\n' => '\u{240a}',
                            _ => c,
                        })
                        .collect();
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

fn is_atext(c: char) -> bool {
    match c {
        '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '/' | '=' | '?' | '^' | '_'
        | '`' | '{' | '|' | '}' | '~' => true,
        c => c.is_ascii_alphanumeric() || is_utf8_non_ascii(c),
    }
}

#[tracable_parser]
fn atext(input: Span) -> IResult<Span, Span> {
    context("atext", take_while1(is_atext))(input)
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

fn is_tspecial(c: char) -> bool {
    match c {
        '(' | ')' | '<' | '>' | '@' | ',' | ';' | ':' | '\\' | '"' | '/' | '[' | ']' | '?'
        | '=' => true,
        _ => false,
    }
}

fn is_attribute_char(c: char) -> bool {
    match c {
        ' ' | '*' | '\'' | '%' => false,
        _ => is_char(c) && !is_ctl(c) && !is_tspecial(c),
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
        let (loc, address) = context("mailbox", addr_spec)(input)?;
        Ok((
            loc,
            Mailbox {
                name: None,
                address,
            },
        ))
    }
}

// address_list = { (address ~ ("," ~ address)*) | obs_addr_list }
#[tracable_parser]
fn address_list(input: Span) -> IResult<Span, AddressList> {
    context(
        "address_list",
        alt((
            map(separated_list1(char(','), address), AddressList),
            obs_address_list,
        )),
    )(input)
}

// obs_addr_list = {  ((cfws? ~ ",")* ~ address ~ ("," ~ (address | cfws))*)+ }
#[tracable_parser]
fn obs_address_list(input: Span) -> IResult<Span, AddressList> {
    let (loc, entries) = context(
        "obs_address_list",
        many1(preceded(
            many0(preceded(opt(cfws), char(','))),
            tuple((
                address,
                many0(preceded(
                    char(','),
                    alt((map(address, |m| Some(m)), map(cfws, |_| None))),
                )),
            )),
        )),
    )(input)?;

    let mut result: Vec<Address> = vec![];

    for (first, boxes) in entries {
        result.push(first);
        for b in boxes {
            if let Some(m) = b {
                result.push(m);
            }
        }
    }

    Ok((loc, AddressList(result)))
}

// address = { mailbox | group }
#[tracable_parser]
fn address(input: Span) -> IResult<Span, Address> {
    context("address", alt((map(mailbox, Address::Mailbox), group)))(input)
}

// group = { display_name ~ ":" ~ group_list? ~ ";" ~ cfws? }
#[tracable_parser]
fn group(input: Span) -> IResult<Span, Address> {
    let (loc, (name, _, group_list, _)) = context(
        "group",
        terminated(
            tuple((display_name, char(':'), opt(group_list), char(';'))),
            opt(cfws),
        ),
    )(input)?;
    Ok((
        loc,
        Address::Group {
            name,
            entries: group_list.unwrap_or_else(|| MailboxList(vec![])),
        },
    ))
}

// group_list = { mailbox_list | cfws | obs_group_list }
#[tracable_parser]
fn group_list(input: Span) -> IResult<Span, MailboxList> {
    context(
        "group_list",
        alt((
            mailbox_list,
            map(cfws, |_| MailboxList(vec![])),
            obs_group_list,
        )),
    )(input)
}

// obs_group_list = @{ (cfws? ~ ",")+ ~ cfws? }
#[tracable_parser]
fn obs_group_list(input: Span) -> IResult<Span, MailboxList> {
    context(
        "obs_group_list",
        map(
            terminated(many1(preceded(opt(cfws), char(','))), opt(cfws)),
            |_| MailboxList(vec![]),
        ),
    )(input)
}

// name_addr = { display_name? ~ angle_addr }
#[tracable_parser]
fn name_addr(input: Span) -> IResult<Span, Mailbox> {
    context(
        "name_addr",
        map(tuple((opt(display_name), angle_addr)), |(name, address)| {
            Mailbox { name, address }
        }),
    )(input)
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
#[tracable_parser]
fn dot_atom_text(input: Span) -> IResult<Span, String> {
    let (loc, (a, b)) = context(
        "dot_atom_text",
        tuple((atext, many0(preceded(char('.'), atext)))),
    )(input)?;
    let mut result = String::new();
    result.push_str(&a);
    for item in b {
        result.push('.');
        result.push_str(&item);
    }

    Ok((loc, result))
}

// dot_atom = { cfws? ~ dot_atom_text ~ cfws? }
#[tracable_parser]
fn dot_atom(input: Span) -> IResult<Span, String> {
    context("dot_atom", delimited(opt(cfws), dot_atom_text, opt(cfws)))(input)
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

// msg_id = { cfws? ~ "<" ~ id_left ~ "@" ~ id_right ~ ">" ~ cfws? }
#[tracable_parser]
fn msg_id(input: Span) -> IResult<Span, String> {
    let (loc, (left, _, right)) = context(
        "msg_id",
        delimited(
            preceded(opt(cfws), char('<')),
            tuple((id_left, char('@'), id_right)),
            preceded(char('>'), opt(cfws)),
        ),
    )(input)?;

    Ok((loc, format!("{left}@{right}")))
}

// msg_id_list = { msg_id+ }
#[tracable_parser]
fn msg_id_list(input: Span) -> IResult<Span, Vec<String>> {
    context("msg_id_list", many1(msg_id))(input)
}

// id_left = { dot_atom_text | obs_id_left }
// obs_id_left = { local_part }
#[tracable_parser]
fn id_left(input: Span) -> IResult<Span, String> {
    context("id_left", alt((dot_atom_text, local_part)))(input)
}

// id_right = { dot_atom_text | no_fold_literal | obs_id_right }
// obs_id_right = { domain }
#[tracable_parser]
fn id_right(input: Span) -> IResult<Span, String> {
    context("id_right", alt((dot_atom_text, no_fold_literal, domain)))(input)
}

// no_fold_literal = { "[" ~ dtext* ~ "]" }
#[tracable_parser]
fn no_fold_literal(input: Span) -> IResult<Span, String> {
    context(
        "no_fold_literal",
        map(
            recognize(tuple((tag("["), take_while(is_dtext), tag("]")))),
            |s: Span| s.to_string(),
        ),
    )(input)
}

// obs_unstruct = { (( "\r"* ~ "\n"* ~ ((encoded_word | obs_utext)~ "\r"* ~ "\n"*)+) | fws)+ }
#[tracable_parser]
fn unstructured(input: Span) -> IResult<Span, String> {
    #[derive(Debug)]
    enum Word {
        Encoded(String),
        UText(char),
        Fws,
    }

    let (loc, words) = context(
        "unstructured",
        many1(alt((
            preceded(
                map(take_while(|c| c == '\r' || c == '\n'), |_| Word::Fws),
                terminated(
                    alt((
                        map(encoded_word, |w| Word::Encoded(w)),
                        map(obs_utext, |c| Word::UText(c)),
                    )),
                    map(take_while(|c| c == '\r' || c == '\n'), |_| Word::Fws),
                ),
            ),
            map(fws, |_| Word::Fws),
        ))),
    )(input)?;

    #[derive(Debug)]
    enum ProcessedWord {
        Encoded(String),
        Text(String),
        Fws,
    }
    let mut processed = vec![];
    for w in words {
        match w {
            Word::Encoded(p) => {
                if processed.len() >= 2
                    && matches!(processed.last(), Some(ProcessedWord::Fws))
                    && matches!(processed[processed.len() - 2], ProcessedWord::Encoded(_))
                {
                    // Fws between encoded words is elided
                    processed.pop();
                }
                processed.push(ProcessedWord::Encoded(p));
            }
            Word::Fws => {
                // Collapse runs of Fws/newline to a single Fws
                if !matches!(processed.last(), Some(ProcessedWord::Fws)) {
                    processed.push(ProcessedWord::Fws);
                }
            }
            Word::UText(c) => match processed.last_mut() {
                Some(ProcessedWord::Text(prior)) => prior.push(c),
                _ => processed.push(ProcessedWord::Text(c.to_string())),
            },
        }
    }

    let mut result = String::new();
    for word in processed {
        match word {
            ProcessedWord::Encoded(s) | ProcessedWord::Text(s) => {
                result.push_str(&s);
            }
            ProcessedWord::Fws => {
                result.push(' ');
            }
        }
    }

    Ok((loc, result))
}

// obs_utext = @{ "\u{00}" | obs_no_ws_ctl | vchar }
#[tracable_parser]
fn obs_utext(input: Span) -> IResult<Span, char> {
    context(
        "obs_utext",
        satisfy(|c| c == '\u{00}' || is_obs_no_ws_ctl(c) || is_vchar(c)),
    )(input)
}

fn is_mime_token(c: char) -> bool {
    is_char(c) && c != ' ' && !is_ctl(c) && !is_tspecial(c)
}

// mime_token = { (!(" " | ctl | tspecials) ~ char)+ }
#[tracable_parser]
fn mime_token(input: Span) -> IResult<Span, Span> {
    context("mime_token", take_while1(is_mime_token))(input)
}

// RFC2045 modified by RFC2231 MIME header fields
// content_type = { cfws? ~ mime_type ~ cfws? ~ "/" ~ cfws? ~ subtype ~
//  cfws? ~ (";"? ~ cfws? ~ parameter ~ cfws?)*
// }
#[tracable_parser]
fn content_type(input: Span) -> IResult<Span, MimeParameters> {
    let (loc, (mime_type, _, _, _, mime_subtype, _, parameters)) = context(
        "content_type",
        preceded(
            opt(cfws),
            tuple((
                mime_token,
                opt(cfws),
                char('/'),
                opt(cfws),
                mime_token,
                opt(cfws),
                many0(preceded(
                    // Note that RFC 2231 is a bit of a mess, showing examples
                    // without `;` as a separator in the original text, but
                    // in the errata from several years later, corrects those
                    // to show the `;`.
                    // In the meantime, there are implementations that assume
                    // that the `;` is optional, so we therefore allow them
                    // to be optional here in our implementation
                    preceded(opt(char(';')), opt(cfws)),
                    terminated(parameter, opt(cfws)),
                )),
            )),
        ),
    )(input)?;

    let value = format!("{mime_type}/{mime_subtype}");
    Ok((loc, MimeParameters { value, parameters }))
}

// parameter = { regular_parameter | extended_parameter }
#[tracable_parser]
fn parameter(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "parameter",
        alt((
            regular_parameter,
            extended_param_with_charset,
            extended_param_no_charset,
        )),
    )(input)
}

#[tracable_parser]
fn extended_param_with_charset(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "extended_param_with_charset",
        map(
            tuple((
                attribute,
                opt(section),
                char('*'),
                opt(cfws),
                char('='),
                opt(cfws),
                opt(mime_charset),
                char('\''),
                opt(mime_language),
                char('\''),
                map(
                    recognize(many0(alt((ext_octet, take_while1(is_attribute_char))))),
                    |s: Span| s.to_string(),
                ),
            )),
            |(name, section, _, _, _, _, mime_charset, _, mime_language, _, value)| MimeParameter {
                name: name.to_string(),
                section,
                mime_charset: mime_charset.map(|s| s.to_string()),
                mime_language: mime_language.map(|s| s.to_string()),
                uses_encoding: true,
                value,
            },
        ),
    )(input)
}

#[tracable_parser]
fn extended_param_no_charset(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "extended_param_no_charset",
        map(
            tuple((
                attribute,
                opt(section),
                opt(char('*')),
                opt(cfws),
                char('='),
                opt(cfws),
                alt((
                    quoted_string,
                    map(
                        recognize(many0(alt((ext_octet, take_while1(is_attribute_char))))),
                        |s: Span| s.to_string(),
                    ),
                )),
            )),
            |(name, section, star, _, _, _, value)| MimeParameter {
                name: name.to_string(),
                section,
                mime_charset: None,
                mime_language: None,
                uses_encoding: star.is_some(),
                value,
            },
        ),
    )(input)
}

#[tracable_parser]
fn mime_charset(input: Span) -> IResult<Span, Span> {
    context(
        "mime_charset",
        take_while1(|c| is_mime_token(c) && c != '\''),
    )(input)
}

#[tracable_parser]
fn mime_language(input: Span) -> IResult<Span, Span> {
    context(
        "mime_language",
        take_while1(|c| is_mime_token(c) && c != '\''),
    )(input)
}

#[tracable_parser]
fn ext_octet(input: Span) -> IResult<Span, Span> {
    context(
        "ext_octet",
        recognize(tuple((
            char('%'),
            satisfy(|c| c.is_ascii_hexdigit()),
            satisfy(|c| c.is_ascii_hexdigit()),
        ))),
    )(input)
}

// section = { "*" ~ ASCII_DIGIT+ }
#[tracable_parser]
fn section(input: Span) -> IResult<Span, u32> {
    context(
        "section",
        preceded(char('*'), nom::character::complete::u32),
    )(input)
}

// regular_parameter = { attribute ~ cfws? ~ "=" ~ cfws? ~ value }
#[tracable_parser]
fn regular_parameter(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "regular_parameter",
        map(
            tuple((attribute, opt(cfws), char('='), opt(cfws), value)),
            |(name, _, _, _, value)| MimeParameter {
                name: name.to_string(),
                value: value,
                section: None,
                uses_encoding: false,
                mime_charset: None,
                mime_language: None,
            },
        ),
    )(input)
}

// attribute = { attribute_char+ }
// attribute_char = { !(" " | ctl | tspecials | "*" | "'" | "%") ~ char }
#[tracable_parser]
fn attribute(input: Span) -> IResult<Span, Span> {
    context("attribute", take_while1(is_attribute_char))(input)
}

#[tracable_parser]
fn value(input: Span) -> IResult<Span, String> {
    context(
        "value",
        alt((map(mime_token, |s: Span| s.to_string()), quoted_string)),
    )(input)
}

pub struct Parser;

impl Parser {
    pub fn parse_mailbox_list_header(text: &str) -> Result<MailboxList> {
        parse_with(text, mailbox_list)
    }

    pub fn parse_mailbox_header(text: &str) -> Result<Mailbox> {
        parse_with(text, mailbox)
    }

    pub fn parse_address_list_header(text: &str) -> Result<AddressList> {
        parse_with(text, address_list)
    }

    pub fn parse_msg_id_header(text: &str) -> Result<String> {
        parse_with(text, msg_id)
    }

    pub fn parse_msg_id_header_list(text: &str) -> Result<Vec<String>> {
        parse_with(text, msg_id_list)
    }

    pub fn parse_content_type_header(text: &str) -> Result<MimeParameters> {
        parse_with(text, content_type)
    }

    pub fn parse_unstructured_header(text: &str) -> Result<String> {
        parse_with(text, unstructured)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddrSpec {
    pub local_part: String,
    pub domain: String,
}

impl AddrSpec {
    pub fn new(local_part: &str, domain: &str) -> Self {
        Self {
            local_part: local_part.to_string(),
            domain: domain.to_string(),
        }
    }
}

impl EncodeHeaderValue for AddrSpec {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = String::new();

        let needs_quoting = !self.local_part.chars().all(|c| is_atext(c) || c == '.');
        if needs_quoting {
            result.push('"');
            // RFC5321 4.1.2 qtextSMTP:
            // within a quoted string, any ASCII graphic or space is permitted without
            // blackslash-quoting except double-quote and the backslash itself.

            for c in self.local_part.chars() {
                if c == '"' || c == '\\' {
                    result.push('\\');
                }
                result.push(c);
            }
            result.push('"');
        } else {
            result.push_str(&self.local_part);
        }
        result.push('@');
        result.push_str(&self.domain);

        result.into()
    }
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
    pub address: AddrSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MimeParameter {
    pub name: String,
    pub section: Option<u32>,
    pub mime_charset: Option<String>,
    pub mime_language: Option<String>,
    pub uses_encoding: bool,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MimeParameters {
    pub value: String,
    parameters: Vec<MimeParameter>,
}

impl MimeParameters {
    /// Retrieve the value for a named parameter.
    /// This method will attempt to decode any %-encoded values
    /// per RFC 2231 and combine multi-element fields into a single
    /// contiguous value.
    /// Invalid charsets and encoding will be silently ignored.
    pub fn get(&self, name: &str) -> String {
        let mut elements: Vec<_> = self
            .parameters
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(name))
            .collect();
        elements.sort_by(|a, b| a.section.cmp(&b.section));

        let mut mime_charset = None;
        let mut result = String::new();

        for ele in elements {
            if let Some(cset) = ele.mime_charset.as_deref() {
                mime_charset = Charset::for_label_no_replacement(cset.as_bytes());
            }

            if let Some((charset, true)) =
                mime_charset.as_ref().map(|cset| (cset, ele.uses_encoding))
            {
                let mut chars = ele.value.chars();
                let mut bytes: Vec<u8> = vec![];

                fn char_to_bytes(c: char, bytes: &mut Vec<u8>) {
                    let mut buf = [0u8; 8];
                    let s = c.encode_utf8(&mut buf);
                    for b in s.bytes() {
                        bytes.push(b);
                    }
                }

                'next_char: while let Some(c) = chars.next() {
                    match c {
                        '%' => {
                            let mut value = 0u8;
                            for _ in 0..2 {
                                match chars.next() {
                                    Some(n) => match n {
                                        '0'..='9' => {
                                            value = value << 4;
                                            value = value | (n as u32 as u8 - b'0');
                                        }
                                        'a'..='f' => {
                                            value = value << 4;
                                            value = value | (n as u32 as u8 - b'a') + 10;
                                        }
                                        'A'..='F' => {
                                            value = value << 4;
                                            value = value | (n as u32 as u8 - b'A') + 10;
                                        }
                                        _ => {
                                            char_to_bytes('%', &mut bytes);
                                            char_to_bytes(n, &mut bytes);
                                            break 'next_char;
                                        }
                                    },
                                    None => {
                                        char_to_bytes('%', &mut bytes);
                                        break 'next_char;
                                    }
                                }
                            }

                            bytes.push(value);
                        }
                        c => {
                            char_to_bytes(c, &mut bytes);
                        }
                    }
                }

                let (decoded, _malformed) = charset.decode_without_bom_handling(&bytes);
                result.push_str(&decoded);
            } else {
                result.push_str(&ele.value);
            }
        }

        result
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
                value.push_str(&self.address.encode_value());
                value.push('>');
                value.into()
            }
            None => format!("<{}>", self.address.encode_value()).into(),
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
                address: AddrSpec {
                    local_part: "someone",
                    domain: "example.com",
                },
            },
            Mailbox {
                name: None,
                address: AddrSpec {
                    local_part: "other",
                    domain: "example.com",
                },
            },
            Mailbox {
                name: Some(
                    "John "Smith" More Quotes",
                ),
                address: AddrSpec {
                    local_part: "someone",
                    domain: "crazy.example.com",
                },
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
        address: AddrSpec {
            local_part: "someone",
            domain: "[127.0.0.1]",
        },
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
                address: AddrSpec {
                    local_part: "someone",
                    domain: "[127.0.0.1]",
                },
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
                address: AddrSpec {
                    local_part: "moore",
                    domain: "cs.utk.edu",
                },
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
                    address: AddrSpec {
                        local_part: "keld",
                        domain: "dkuug.dk",
                    },
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
                    address: AddrSpec {
                        local_part: "PIRARD",
                        domain: "vm1.ulg.ac.be",
                    },
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
                address: AddrSpec {
                    local_part: "moore",
                    domain: "cs.utk.edu",
                },
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
                    address: AddrSpec {
                        local_part: "keld",
                        domain: "dkuug.dk",
                    },
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
                    address: AddrSpec {
                        local_part: "PIRARD",
                        domain: "vm1.ulg.ac.be",
                    },
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
                        address: AddrSpec {
                            local_part: "c",
                            domain: "a.test",
                        },
                    },
                    Mailbox {
                        name: None,
                        address: AddrSpec {
                            local_part: "joe",
                            domain: "where.test",
                        },
                    },
                    Mailbox {
                        name: Some(
                            "John",
                        ),
                        address: AddrSpec {
                            local_part: "jdoe",
                            domain: "one.test",
                        },
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
            Ok(params) => params.unwrap(),
        };

        k9::snapshot!(params.get("charset"), "us-ascii");
        k9::snapshot!(
            params,
            r#"
MimeParameters {
    value: "text/plain",
    parameters: [
        MimeParameter {
            name: "charset",
            section: None,
            mime_charset: None,
            mime_language: None,
            uses_encoding: false,
            value: "us-ascii",
        },
    ],
}
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
            MimeParameter {
                name: "charset",
                section: None,
                mime_charset: None,
                mime_language: None,
                uses_encoding: false,
                value: "us-ascii",
            },
        ],
    },
)
"#
        );
    }

    #[test]
    fn content_type_rfc2231() {
        // This example is taken from the errata for rfc2231.
        // <https://www.rfc-editor.org/errata/eid590>
        let message = concat!(
            "Content-Type: application/x-stuff;\n",
            "\ttitle*0*=us-ascii'en'This%20is%20even%20more%20;\n",
            "\ttitle*1*=%2A%2A%2Afun%2A%2A%2A%20;\n",
            "\ttitle*2=\"isn't it!\"\n",
            "\n\n\n"
        );
        let msg = MimePart::parse(message).unwrap();
        let params = match msg.headers().content_type() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(params) => params.unwrap(),
        };
        k9::snapshot!(
            params.get("title"),
            r#"This is even more ***fun*** isn't it!"#
        );

        k9::snapshot!(
            params,
            r#"
MimeParameters {
    value: "application/x-stuff",
    parameters: [
        MimeParameter {
            name: "title",
            section: Some(
                0,
            ),
            mime_charset: Some(
                "us-ascii",
            ),
            mime_language: Some(
                "en",
            ),
            uses_encoding: true,
            value: "This%20is%20even%20more%20",
        },
        MimeParameter {
            name: "title",
            section: Some(
                1,
            ),
            mime_charset: None,
            mime_language: None,
            uses_encoding: true,
            value: "%2A%2A%2Afun%2A%2A%2A%20",
        },
        MimeParameter {
            name: "title",
            section: Some(
                2,
            ),
            mime_charset: None,
            mime_language: None,
            uses_encoding: false,
            value: "isn't it!",
        },
    ],
}
"#
        );
    }
}
