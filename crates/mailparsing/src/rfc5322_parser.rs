use crate::headermap::EncodeHeaderValue;
use crate::nom_utils::{explain_nom, make_context_error, make_span, IResult, ParseError, Span};
use crate::{MailParsingError, Result, SharedString};
use charset::Charset;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while, take_while1};
use nom::character::complete::{char, satisfy};
use nom::combinator::{all_consuming, map, opt, recognize};
use nom::error::context;
use nom::multi::{many0, many1, separated_list1};
use nom::sequence::{delimited, preceded, separated_pair, terminated, tuple};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Debug;

impl MailParsingError {
    pub fn from_nom(input: Span, err: nom::Err<ParseError<Span<'_>>>) -> Self {
        MailParsingError::HeaderParse(explain_nom(input, err))
    }
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

fn wsp(input: Span) -> IResult<Span, Span> {
    context("wsp", take_while1(|c| c == ' ' || c == '\t'))(input)
}

fn newline(input: Span) -> IResult<Span, Span> {
    context("newline", recognize(preceded(opt(char('\r')), char('\n'))))(input)
}

// fws = { ((wsp* ~ "\r"? ~ "\n")* ~ wsp+) | obs_fws }
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
fn obs_fws(input: Span) -> IResult<Span, Span> {
    context(
        "obs_fws",
        recognize(preceded(many1(wsp), preceded(newline, many1(wsp)))),
    )(input)
}

// mailbox_list = { (mailbox ~ ("," ~ mailbox)*) | obs_mbox_list }
fn mailbox_list(input: Span) -> IResult<Span, MailboxList> {
    let (loc, mailboxes) = context(
        "mailbox_list",
        alt((separated_list1(char(','), mailbox), obs_mbox_list)),
    )(input)?;
    Ok((loc, MailboxList(mailboxes)))
}

// obs_mbox_list = {  ((cfws? ~ ",")* ~ mailbox ~ ("," ~ (mailbox | cfws))*)+ }
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
fn address(input: Span) -> IResult<Span, Address> {
    context("address", alt((map(mailbox, Address::Mailbox), group)))(input)
}

// group = { display_name ~ ":" ~ group_list? ~ ";" ~ cfws? }
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
fn name_addr(input: Span) -> IResult<Span, Mailbox> {
    context(
        "name_addr",
        map(tuple((opt(display_name), angle_addr)), |(name, address)| {
            Mailbox { name, address }
        }),
    )(input)
}

// display_name = { phrase }
fn display_name(input: Span) -> IResult<Span, String> {
    context("display_name", phrase)(input)
}

// phrase = { (encoded_word | word)+ | obs_phrase }
// obs_phrase = { (encoded_word | word) ~ (encoded_word | word | dot | cfws)* }
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
       ^___________________________
expected '@', found .

1: at line 1, in addr_spec:
"darth".vader@a.galaxy.far.far.away
^__________________________________

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
fn local_part(input: Span) -> IResult<Span, String> {
    context("local_part", alt((dot_atom, quoted_string, obs_local_part)))(input)
}

// domain = { dot_atom | domain_literal | obs_domain }
fn domain(input: Span) -> IResult<Span, String> {
    context("domain", alt((dot_atom, domain_literal, obs_domain)))(input)
}

// obs_domain = { atom ~ ( dot ~ atom)* }
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
        extra: (),
    },
)
"#
    );
}

// ccontent = { ctext | quoted_pair | comment | encoded_word }
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

fn is_quoted_pair(c: char) -> bool {
    match c {
        '\u{00}' | '\r' | '\n' | ' ' => true,
        c => is_obs_no_ws_ctl(c) || is_vchar(c),
    }
}

// quoted_pair = { ( "\\"  ~ (vchar | wsp)) | obs_qp }
// obs_qp = { "\\" ~ ( "\u{00}" | obs_no_ws_ctl | "\r" | "\n") }
fn quoted_pair(input: Span) -> IResult<Span, char> {
    context("quoted_pair", preceded(char('\\'), satisfy(is_quoted_pair)))(input)
}

// encoded_word = { "=?" ~ charset ~ ("*" ~ language)? ~ "?" ~ encoding ~ "?" ~ encoded_text ~ "?=" }
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
            .map_err(|err| {
                make_context_error(
                    input,
                    format!("encoded_word: base64 decode failed: {err:#}"),
                )
            })?,
        "Q" | "q" => {
            quoted_printable::decode(text.replace("_", " "), quoted_printable::ParseMode::Robust)
                .map_err(|err| {
                    make_context_error(
                        input,
                        format!("encoded_word: quoted printable decode failed: {err:#}"),
                    )
                })?
        }
        encoding => {
            return Err(make_context_error(
                input,
                format!(
                    "encoded_word: invalid encoding '{encoding}', expected one of b, B, q or Q"
                ),
            ));
        }
    };

    let charset = Charset::for_label_no_replacement(charset.as_bytes()).ok_or_else(|| {
        make_context_error(
            input,
            format!("encoded_word: unsupported charset '{charset}'"),
        )
    })?;

    let (decoded, _malformed) = charset.decode_without_bom_handling(&bytes);

    Ok((loc, decoded.to_string()))
}

// charset = @{ (!"*" ~ token)+ }
fn charset(input: Span) -> IResult<Span, Span> {
    context("charset", take_while1(|c| c != '*' && is_token(c)))(input)
}

// language = @{ token+ }
fn language(input: Span) -> IResult<Span, Span> {
    context("language", take_while1(|c| c != '*' && is_token(c)))(input)
}

// encoding = @{ token+ }
fn encoding(input: Span) -> IResult<Span, Span> {
    context("encoding", take_while1(|c| c != '*' && is_token(c)))(input)
}

// encoded_text = @{ (!( " " | "?") ~ vchar)+ }
fn encoded_text(input: Span) -> IResult<Span, Span> {
    context(
        "encoded_text",
        take_while1(|c| is_vchar(c) && c != ' ' && c != '?'),
    )(input)
}

// quoted_string = { cfws? ~ "\"" ~ (fws? ~ qcontent)* ~ fws? ~ "\"" ~ cfws? }
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
fn qcontent(input: Span) -> IResult<Span, char> {
    context("qcontent", alt((satisfy(is_qtext), quoted_pair)))(input)
}

// msg_id = { cfws? ~ "<" ~ id_left ~ "@" ~ id_right ~ ">" ~ cfws? }
fn msg_id(input: Span) -> IResult<Span, MessageID> {
    let (loc, (left, _, right)) = context(
        "msg_id",
        delimited(
            preceded(opt(cfws), char('<')),
            tuple((id_left, char('@'), id_right)),
            preceded(char('>'), opt(cfws)),
        ),
    )(input)?;

    Ok((loc, MessageID(format!("{left}@{right}"))))
}

fn content_id(input: Span) -> IResult<Span, MessageID> {
    let (loc, id) = context(
        "content_id",
        delimited(
            preceded(opt(cfws), char('<')),
            id_right,
            preceded(char('>'), opt(cfws)),
        ),
    )(input)?;

    Ok((loc, MessageID(id)))
}

// msg_id_list = { msg_id+ }
fn msg_id_list(input: Span) -> IResult<Span, Vec<MessageID>> {
    context("msg_id_list", many1(msg_id))(input)
}

// id_left = { dot_atom_text | obs_id_left }
// obs_id_left = { local_part }
fn id_left(input: Span) -> IResult<Span, String> {
    context("id_left", alt((dot_atom_text, local_part)))(input)
}

// id_right = { dot_atom_text | no_fold_literal | obs_id_right }
// obs_id_right = { domain }
fn id_right(input: Span) -> IResult<Span, String> {
    context("id_right", alt((dot_atom_text, no_fold_literal, domain)))(input)
}

// no_fold_literal = { "[" ~ dtext* ~ "]" }
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
fn unstructured(input: Span) -> IResult<Span, String> {
    #[derive(Debug)]
    enum Word {
        Encoded(String),
        UText(char),
        Fws,
    }

    let (loc, words) = context(
        "unstructured",
        many0(alt((
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

fn authentication_results(input: Span) -> IResult<Span, AuthenticationResults> {
    context(
        "authentication_results",
        map(
            tuple((
                opt(cfws),
                value,
                opt(preceded(cfws, nom::character::complete::u32)),
                alt((no_result, many1(resinfo))),
                opt(cfws),
            )),
            |(_, serv_id, version, results, _)| AuthenticationResults {
                serv_id,
                version,
                results,
            },
        ),
    )(input)
}

fn no_result(input: Span) -> IResult<Span, Vec<AuthenticationResult>> {
    context(
        "no_result",
        map(
            tuple((opt(cfws), char(';'), opt(cfws), tag("none"))),
            |_| vec![],
        ),
    )(input)
}

fn resinfo(input: Span) -> IResult<Span, AuthenticationResult> {
    context(
        "resinfo",
        map(
            tuple((
                opt(cfws),
                char(';'),
                methodspec,
                opt(preceded(cfws, reasonspec)),
                opt(many1(propspec)),
            )),
            |(_, _, (method, method_version, result), reason, props)| AuthenticationResult {
                method,
                method_version,
                result,
                reason,
                props: match props {
                    None => BTreeMap::default(),
                    Some(props) => props.into_iter().collect(),
                },
            },
        ),
    )(input)
}

fn methodspec(input: Span) -> IResult<Span, (String, Option<u32>, String)> {
    context(
        "methodspec",
        map(
            tuple((
                opt(cfws),
                tuple((keyword, opt(methodversion))),
                opt(cfws),
                char('='),
                opt(cfws),
                keyword,
            )),
            |(_, (method, methodversion), _, _, _, result)| (method, methodversion, result),
        ),
    )(input)
}

// Taken from https://datatracker.ietf.org/doc/html/rfc8601 which says
// that this is the same as the SMTP Keyword token
fn keyword(input: Span) -> IResult<Span, String> {
    context(
        "keyword",
        map(
            take_while1(|c: char| c.is_ascii_alphanumeric() || c == '+'),
            |s: Span| s.to_string(),
        ),
    )(input)
}

fn methodversion(input: Span) -> IResult<Span, u32> {
    context(
        "methodversion",
        preceded(
            tuple((opt(cfws), char('/'), opt(cfws))),
            nom::character::complete::u32,
        ),
    )(input)
}

fn reasonspec(input: Span) -> IResult<Span, String> {
    context(
        "reason",
        map(
            tuple((tag("reason"), opt(cfws), char('='), opt(cfws), value)),
            |(_, _, _, _, value)| value,
        ),
    )(input)
}

fn propspec(input: Span) -> IResult<Span, (String, String)> {
    context(
        "propspec",
        map(
            tuple((
                opt(cfws),
                keyword,
                opt(cfws),
                char('.'),
                opt(cfws),
                keyword,
                opt(cfws),
                char('='),
                opt(cfws),
                alt((
                    map(preceded(char('@'), domain), |d| format!("@{d}")),
                    map(separated_pair(local_part, char('@'), domain), |(u, d)| {
                        format!("{u}@{d}")
                    }),
                    domain,
                    // value must be last in this alternation
                    value,
                )),
                opt(cfws),
            )),
            |(_, ptype, _, _, _, property, _, _, _, value, _)| {
                (format!("{ptype}.{property}"), value)
            },
        ),
    )(input)
}

// obs_utext = @{ "\u{00}" | obs_no_ws_ctl | vchar }
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
fn mime_token(input: Span) -> IResult<Span, Span> {
    context("mime_token", take_while1(is_mime_token))(input)
}

// RFC2045 modified by RFC2231 MIME header fields
// content_type = { cfws? ~ mime_type ~ cfws? ~ "/" ~ cfws? ~ subtype ~
//  cfws? ~ (";"? ~ cfws? ~ parameter ~ cfws?)*
// }
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

fn content_transfer_encoding(input: Span) -> IResult<Span, MimeParameters> {
    let (loc, (value, _, parameters)) = context(
        "content_type",
        preceded(
            opt(cfws),
            tuple((
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

    Ok((
        loc,
        MimeParameters {
            value: value.to_string(),
            parameters,
        },
    ))
}

// parameter = { regular_parameter | extended_parameter }
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

fn mime_charset(input: Span) -> IResult<Span, Span> {
    context(
        "mime_charset",
        take_while1(|c| is_mime_token(c) && c != '\''),
    )(input)
}

fn mime_language(input: Span) -> IResult<Span, Span> {
    context(
        "mime_language",
        take_while1(|c| is_mime_token(c) && c != '\''),
    )(input)
}

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
fn section(input: Span) -> IResult<Span, u32> {
    context(
        "section",
        preceded(char('*'), nom::character::complete::u32),
    )(input)
}

// regular_parameter = { attribute ~ cfws? ~ "=" ~ cfws? ~ value }
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
fn attribute(input: Span) -> IResult<Span, Span> {
    context("attribute", take_while1(is_attribute_char))(input)
}

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

    pub fn parse_msg_id_header(text: &str) -> Result<MessageID> {
        parse_with(text, msg_id)
    }

    pub fn parse_msg_id_header_list(text: &str) -> Result<Vec<MessageID>> {
        parse_with(text, msg_id_list)
    }

    pub fn parse_content_id_header(text: &str) -> Result<MessageID> {
        parse_with(text, content_id)
    }

    pub fn parse_content_type_header(text: &str) -> Result<MimeParameters> {
        parse_with(text, content_type)
    }

    pub fn parse_content_transfer_encoding_header(text: &str) -> Result<MimeParameters> {
        parse_with(text, content_transfer_encoding)
    }

    pub fn parse_unstructured_header(text: &str) -> Result<String> {
        parse_with(text, unstructured)
    }

    pub fn parse_authentication_results_header(text: &str) -> Result<AuthenticationResults> {
        parse_with(text, authentication_results)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticationResults {
    pub serv_id: String,
    pub version: Option<u32>,
    pub results: Vec<AuthenticationResult>,
}

/// Emits a value that was parsed by `value`, into target
fn emit_value_token(value: &str, target: &mut String) {
    let use_quoted_string = !value.chars().all(|c| is_mime_token(c) || c == '@');
    if use_quoted_string {
        target.push('"');
        for c in value.chars() {
            if c == '"' || c == '\\' {
                target.push('\\');
            }
            target.push(c);
        }
        target.push('"');
    } else {
        target.push_str(value);
    }
}

impl EncodeHeaderValue for AuthenticationResults {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = match self.version {
            Some(v) => format!("{} {v}", self.serv_id),
            None => format!("{}", self.serv_id),
        };
        if self.results.is_empty() {
            result.push_str("; none");
        } else {
            for res in &self.results {
                result.push_str(";\r\n\t");
                emit_value_token(&res.method, &mut result);
                if let Some(v) = res.method_version {
                    result.push_str(&format!("/{v}"));
                }
                result.push('=');
                emit_value_token(&res.result, &mut result);
                if let Some(reason) = &res.reason {
                    result.push_str(" reason=");
                    emit_value_token(reason, &mut result);
                }
                for (k, v) in &res.props {
                    result.push_str(&format!("\r\n\t{k}="));
                    emit_value_token(v, &mut result);
                }
            }
        }

        result.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticationResult {
    pub method: String,
    pub method_version: Option<u32>,
    pub result: String,
    pub reason: Option<String>,
    pub props: BTreeMap<String, String>,
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

    pub fn parse(email: &str) -> Result<Self> {
        parse_with(email, addr_spec)
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
pub struct AddressList(pub Vec<Address>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxList(pub Vec<Mailbox>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    pub name: Option<String>,
    pub address: AddrSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageID(pub String);

impl EncodeHeaderValue for MessageID {
    fn encode_value(&self) -> SharedString<'static> {
        format!("<{}>", self.0).into()
    }
}

impl EncodeHeaderValue for Vec<MessageID> {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = String::new();
        for id in self {
            if !result.is_empty() {
                result.push_str("\r\n\t");
            }
            result.push_str(&format!("<{}>", id.0));
        }
        result.into()
    }
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
    pub fn new(value: &str) -> Self {
        Self {
            value: value.to_string(),
            parameters: vec![],
        }
    }

    /// Retrieve the value for a named parameter.
    /// This method will attempt to decode any %-encoded values
    /// per RFC 2231 and combine multi-element fields into a single
    /// contiguous value.
    /// Invalid charsets and encoding will be silently ignored.
    pub fn get(&self, name: &str) -> Option<String> {
        let mut elements: Vec<_> = self
            .parameters
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(name))
            .collect();
        if elements.is_empty() {
            return None;
        }
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

        Some(result)
    }

    /// Remove the named parameter
    pub fn remove(&mut self, name: &str) {
        self.parameters
            .retain(|p| !p.name.eq_ignore_ascii_case(name));
    }

    pub fn set(&mut self, name: &str, value: &str) {
        self.remove(name);

        self.parameters.push(MimeParameter {
            name: name.to_string(),
            value: value.to_string(),
            section: None,
            mime_charset: None,
            mime_language: None,
            uses_encoding: false,
        });
    }

    pub fn is_multipart(&self) -> bool {
        self.value.starts_with("message/") || self.value.starts_with("multipart/")
    }

    pub fn is_text(&self) -> bool {
        self.value.starts_with("text/")
    }
}

impl EncodeHeaderValue for MimeParameters {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = self.value.to_string();
        let mut names: Vec<&str> = self.parameters.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        names.dedup();

        for name in names {
            let value = self.get(name).expect("name to be present");

            let needs_encoding = value.chars().any(|c| !is_mime_token(c) || !c.is_ascii());
            // Prefer to use quoted_string representation when possible, as it doesn't
            // require any RFC 2231 encoding
            let use_quoted_string = value
                .chars()
                .all(|c| (is_qtext(c) || is_quoted_pair(c)) && c.is_ascii());

            let mut params = vec![];
            let mut chars = value.chars().peekable();
            while chars.peek().is_some() {
                let count = params.len();
                let is_first = count == 0;
                let prefix = if use_quoted_string {
                    "\""
                } else if is_first && needs_encoding {
                    "UTF-8''"
                } else {
                    ""
                };
                let limit = 74 - (name.len() + 4 + prefix.len());

                let mut encoded = String::new();

                while encoded.len() < limit {
                    let c = match chars.next() {
                        Some(c) => c,
                        None => break,
                    };

                    if use_quoted_string {
                        if c == '"' || c == '\\' {
                            encoded.push('\\');
                        }
                        encoded.push(c);
                    } else if is_mime_token(c) && (!needs_encoding || c != '%') {
                        encoded.push(c);
                    } else {
                        let mut buf = [0u8; 8];
                        let s = c.encode_utf8(&mut buf);
                        for b in s.bytes() {
                            encoded.push('%');
                            encoded.push(HEX_CHARS[(b as usize) >> 4] as char);
                            encoded.push(HEX_CHARS[(b as usize) & 0x0f] as char);
                        }
                    }
                }

                if use_quoted_string {
                    encoded.push('"');
                }

                params.push(MimeParameter {
                    name: name.to_string(),
                    section: Some(count as u32),
                    mime_charset: if is_first {
                        Some("UTF-8".to_string())
                    } else {
                        None
                    },
                    mime_language: None,
                    uses_encoding: needs_encoding,
                    value: encoded,
                })
            }
            if params.len() == 1 {
                params.last_mut().map(|p| p.section = None);
            }
            for p in params {
                result.push_str(";\r\n\t");
                let charset_tick = if !use_quoted_string
                    && (p.mime_charset.is_some() || p.mime_language.is_some())
                {
                    "'"
                } else {
                    ""
                };
                let lang_tick = if !use_quoted_string
                    && (p.mime_language.is_some() || p.mime_charset.is_some())
                {
                    "'"
                } else {
                    ""
                };

                let section = p
                    .section
                    .map(|s| format!("*{s}"))
                    .unwrap_or_else(|| String::new());

                let uses_encoding = if !use_quoted_string && p.uses_encoding {
                    "*"
                } else {
                    ""
                };
                let charset = if use_quoted_string {
                    "\""
                } else {
                    p.mime_charset.as_deref().unwrap_or("")
                };
                let lang = p.mime_language.as_deref().unwrap_or("");

                let line = format!(
                    "{name}{section}{uses_encoding}={charset}{charset_tick}{lang}{lang_tick}{value}",
                    name = &p.name,
                    value = &p.value
                );
                result.push_str(&line);
            }
        }
        result.into()
    }
}

static HEX_CHARS: &[u8] = &[
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'A', b'B', b'C', b'D', b'E', b'F',
];

pub(crate) fn qp_encode(s: &str) -> String {
    let prefix = b"=?UTF-8?q?";
    let suffix = b"?=";
    let limit = 74 - (prefix.len() + suffix.len());

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
    fn docomo_non_compliant_localpart() {
        let message = "Sender: hello..there@docomo.ne.jp\n\n\n";
        let msg = MimePart::parse(message).unwrap();
        let err = msg.headers().sender().unwrap_err();
        k9::snapshot!(
            err,
            r#"
HeaderParse(
    "0: at line 1:
hello..there@docomo.ne.jp
     ^___________________
expected '@', found .

1: at line 1, in addr_spec:
hello..there@docomo.ne.jp
^________________________

2: at line 1, in mailbox:
hello..there@docomo.ne.jp
^________________________

",
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
    fn rfc2047_bogus() {
        let message = concat!(
            "From: =?US-OSCII?Q?Keith_Moore?= <moore@cs.utk.edu>\n",
            "To: =?ISO-8859-1*en-us?Q?Keld_J=F8rn_Simonsen?= <keld@dkuug.dk>\n",
            "CC: =?ISO-8859-1?Q?Andr=E?= Pirard <PIRARD@vm1.ulg.ac.be>\n",
            "Subject: Hello =?ISO-8859-1?B?SWYgeW91IGNhb!ByZWFkIHRoaXMgeW8=?=\n",
            "  =?ISO-8859-2?B?dSB1bmRlcnN0YW5kIHRoZSBleGFtcGxlLg==?=\n",
            "\n\n"
        );
        let msg = MimePart::parse(message).unwrap();

        // Invalid charset causes encoded_word to fail and we will instead match
        // obs_utext and return it as it was
        k9::assert_equal!(
            msg.headers().from().unwrap().unwrap().0[0]
                .name
                .as_ref()
                .unwrap(),
            "=?US-OSCII?Q?Keith_Moore?="
        );

        match &msg.headers().cc().unwrap().unwrap().0[0] {
            Address::Mailbox(mbox) => {
                // 'Andr=E9?=' is in the non-bogus example below, but above we
                // broke it as 'Andr=E?=', and instead of triggering a qp decode
                // error, it is passed through here as-is
                k9::assert_equal!(mbox.name.as_ref().unwrap(), "Andr=E Pirard");
            }
            wat => panic!("should not have {wat:?}"),
        }

        // The invalid base64 (an I was replaced by an !) is interpreted as obs_utext
        // and passed through to us
        k9::assert_equal!(
            msg.headers().subject().unwrap().unwrap(),
            "Hello =?ISO-8859-1?B?SWYgeW91IGNhb!ByZWFkIHRoaXMgeW8=?= u understand the example."
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

        k9::snapshot!(
            msg.rebuild().unwrap().to_message_string(),
            r#"
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Content-Transfer-Encoding: quoted-printable\r
From: Keith Moore <moore@cs.utk.edu>\r
To: =?UTF-8?q?Keld_J=C3=B8rn_Simonsen?= <keld@dkuug.dk>\r
Cc: =?UTF-8?q?Andr=C3=A9_Pirard?= <PIRARD@vm1.ulg.ac.be>\r
Subject: Hello If you can read this you understand the example.\r
\r
=0A\r

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
    MessageID(
        "foo@example.com",
    ),
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
        MessageID(
            "a@example.com",
        ),
        MessageID(
            "b@example.com",
        ),
        MessageID(
            "legacy@example.com",
        ),
        MessageID(
            "literal@[127.0.0.1]",
        ),
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

        k9::snapshot!(
            params.get("charset"),
            r#"
Some(
    "us-ascii",
)
"#
        );
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
        let mut params = match msg.headers().content_type() {
            Err(err) => panic!("Doh.\n{err:#}"),
            Ok(params) => params.unwrap(),
        };

        let original_title = params.get("title");
        k9::snapshot!(
            &original_title,
            r#"
Some(
    "This is even more ***fun*** isn't it!",
)
"#
        );

        k9::snapshot!(
            &params,
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

        k9::snapshot!(
            params.encode_value(),
            r#"
application/x-stuff;\r
\ttitle="This is even more ***fun*** isn't it!"
"#
        );

        params.set("foo", "bar 💩");

        params.set(
            "long",
            "this is some text that should wrap because \
                it should be a good bit longer than our target maximum \
                length for this sort of thing, and hopefully we see at \
                least three lines produced as a result of setting \
                this value in this way",
        );

        params.set(
            "longernnamethananyoneshouldreallyuse",
            "this is some text that should wrap because \
                it should be a good bit longer than our target maximum \
                length for this sort of thing, and hopefully we see at \
                least three lines produced as a result of setting \
                this value in this way",
        );

        k9::snapshot!(
            params.encode_value(),
            r#"
application/x-stuff;\r
\tfoo*=UTF-8''bar%20%F0%9F%92%A9;\r
\tlong*0="this is some text that should wrap because it should be a good bi";\r
\tlong*1="t longer than our target maximum length for this sort of thing, a";\r
\tlong*2="nd hopefully we see at least three lines produced as a result of ";\r
\tlong*3="setting this value in this way";\r
\tlongernnamethananyoneshouldreallyuse*0="this is some text that should wra";\r
\tlongernnamethananyoneshouldreallyuse*1="p because it should be a good bit";\r
\tlongernnamethananyoneshouldreallyuse*2=" longer than our target maximum l";\r
\tlongernnamethananyoneshouldreallyuse*3="ength for this sort of thing, and";\r
\tlongernnamethananyoneshouldreallyuse*4=" hopefully we see at least three ";\r
\tlongernnamethananyoneshouldreallyuse*5="lines produced as a result of set";\r
\tlongernnamethananyoneshouldreallyuse*6="ting this value in this way";\r
\ttitle="This is even more ***fun*** isn't it!"
"#
        );
    }

    /// <https://datatracker.ietf.org/doc/html/rfc8601#appendix-B.2>
    #[test]
    fn authentication_results_b_2() {
        let ar = Header::with_name_value("Authentication-Results", "example.org 1; none");
        let ar = ar.as_authentication_results().unwrap();
        k9::snapshot!(
            &ar,
            r#"
AuthenticationResults {
    serv_id: "example.org",
    version: Some(
        1,
    ),
    results: [],
}
"#
        );

        k9::snapshot!(ar.encode_value(), "example.org 1; none");
    }

    /// <https://datatracker.ietf.org/doc/html/rfc8601#appendix-B.3>
    #[test]
    fn authentication_results_b_3() {
        let ar = Header::with_name_value(
            "Authentication-Results",
            "example.com; spf=pass smtp.mailfrom=example.net",
        );
        k9::snapshot!(
            ar.as_authentication_results(),
            r#"
Ok(
    AuthenticationResults {
        serv_id: "example.com",
        version: None,
        results: [
            AuthenticationResult {
                method: "spf",
                method_version: None,
                result: "pass",
                reason: None,
                props: {
                    "smtp.mailfrom": "example.net",
                },
            },
        ],
    },
)
"#
        );
    }

    /// <https://datatracker.ietf.org/doc/html/rfc8601#appendix-B.4>
    #[test]
    fn authentication_results_b_4() {
        let ar = Header::with_name_value(
            "Authentication-Results",
            concat!(
                "example.com;\n",
                "\tauth=pass (cram-md5) smtp.auth=sender@example.net;\n",
                "\tspf=pass smtp.mailfrom=example.net"
            ),
        );
        k9::snapshot!(
            ar.as_authentication_results(),
            r#"
Ok(
    AuthenticationResults {
        serv_id: "example.com",
        version: None,
        results: [
            AuthenticationResult {
                method: "auth",
                method_version: None,
                result: "pass",
                reason: None,
                props: {
                    "smtp.auth": "sender@example.net",
                },
            },
            AuthenticationResult {
                method: "spf",
                method_version: None,
                result: "pass",
                reason: None,
                props: {
                    "smtp.mailfrom": "example.net",
                },
            },
        ],
    },
)
"#
        );

        let ar = Header::with_name_value(
            "Authentication-Results",
            "example.com; iprev=pass\n\tpolicy.iprev=192.0.2.200",
        );
        k9::snapshot!(
            ar.as_authentication_results(),
            r#"
Ok(
    AuthenticationResults {
        serv_id: "example.com",
        version: None,
        results: [
            AuthenticationResult {
                method: "iprev",
                method_version: None,
                result: "pass",
                reason: None,
                props: {
                    "policy.iprev": "192.0.2.200",
                },
            },
        ],
    },
)
"#
        );
    }

    /// <https://datatracker.ietf.org/doc/html/rfc8601#appendix-B.5>
    #[test]
    fn authentication_results_b_5() {
        let ar = Header::with_name_value(
            "Authentication-Results",
            "example.com;\n\tdkim=pass (good signature) header.d=example.com",
        );
        k9::snapshot!(
            ar.as_authentication_results(),
            r#"
Ok(
    AuthenticationResults {
        serv_id: "example.com",
        version: None,
        results: [
            AuthenticationResult {
                method: "dkim",
                method_version: None,
                result: "pass",
                reason: None,
                props: {
                    "header.d": "example.com",
                },
            },
        ],
    },
)
"#
        );

        let ar = Header::with_name_value(
            "Authentication-Results",
            "example.com;\n\tauth=pass (cram-md5) smtp.auth=sender@example.com;\n\tspf=fail smtp.mailfrom=example.com"
        );
        let ar = ar.as_authentication_results().unwrap();
        k9::snapshot!(
            &ar,
            r#"
AuthenticationResults {
    serv_id: "example.com",
    version: None,
    results: [
        AuthenticationResult {
            method: "auth",
            method_version: None,
            result: "pass",
            reason: None,
            props: {
                "smtp.auth": "sender@example.com",
            },
        },
        AuthenticationResult {
            method: "spf",
            method_version: None,
            result: "fail",
            reason: None,
            props: {
                "smtp.mailfrom": "example.com",
            },
        },
    ],
}
"#
        );

        k9::snapshot!(
            ar.encode_value(),
            r#"
example.com;\r
\tauth=pass\r
\tsmtp.auth=sender@example.com;\r
\tspf=fail\r
\tsmtp.mailfrom=example.com
"#
        );
    }

    /// <https://datatracker.ietf.org/doc/html/rfc8601#appendix-B.6>
    #[test]
    fn authentication_results_b_6() {
        let ar = Header::with_name_value(
            "Authentication-Results",
            concat!(
                "example.com;\n",
                "\tdkim=pass reason=\"good signature\"\n",
                "\theader.i=@mail-router.example.net;\n",
                "\tdkim=fail reason=\"bad signature\"\n",
                "\theader.i=@newyork.example.com"
            ),
        );
        let ar = match ar.as_authentication_results() {
            Err(err) => panic!("\n{err}"),
            Ok(ar) => ar,
        };

        k9::snapshot!(
            &ar,
            r#"
AuthenticationResults {
    serv_id: "example.com",
    version: None,
    results: [
        AuthenticationResult {
            method: "dkim",
            method_version: None,
            result: "pass",
            reason: Some(
                "good signature",
            ),
            props: {
                "header.i": "@mail-router.example.net",
            },
        },
        AuthenticationResult {
            method: "dkim",
            method_version: None,
            result: "fail",
            reason: Some(
                "bad signature",
            ),
            props: {
                "header.i": "@newyork.example.com",
            },
        },
    ],
}
"#
        );

        k9::snapshot!(
            ar.encode_value(),
            r#"
example.com;\r
\tdkim=pass reason="good signature"\r
\theader.i=@mail-router.example.net;\r
\tdkim=fail reason="bad signature"\r
\theader.i=@newyork.example.com
"#
        );

        let ar = Header::with_name_value(
            "Authentication-Results",
            concat!(
                "example.net;\n",
                "\tdkim=pass (good signature) header.i=@newyork.example.com"
            ),
        );
        let ar = match ar.as_authentication_results() {
            Err(err) => panic!("\n{err}"),
            Ok(ar) => ar,
        };

        k9::snapshot!(
            &ar,
            r#"
AuthenticationResults {
    serv_id: "example.net",
    version: None,
    results: [
        AuthenticationResult {
            method: "dkim",
            method_version: None,
            result: "pass",
            reason: None,
            props: {
                "header.i": "@newyork.example.com",
            },
        },
    ],
}
"#
        );

        k9::snapshot!(
            ar.encode_value(),
            r#"
example.net;\r
\tdkim=pass\r
\theader.i=@newyork.example.com
"#
        );
    }

    /// <https://datatracker.ietf.org/doc/html/rfc8601#appendix-B.7>
    #[test]
    fn authentication_results_b_7() {
        let ar = Header::with_name_value(
            "Authentication-Results",
            concat!(
                "foo.example.net (foobar) 1 (baz);\n",
                "\tdkim (Because I like it) / 1 (One yay) = (wait for it) fail\n",
                "\tpolicy (A dot can go here) . (like that) expired\n",
                "\t(this surprised me) = (as I wasn't expecting it) 1362471462"
            ),
        );
        let ar = match ar.as_authentication_results() {
            Err(err) => panic!("\n{err}"),
            Ok(ar) => ar,
        };

        k9::snapshot!(
            &ar,
            r#"
AuthenticationResults {
    serv_id: "foo.example.net",
    version: Some(
        1,
    ),
    results: [
        AuthenticationResult {
            method: "dkim",
            method_version: Some(
                1,
            ),
            result: "fail",
            reason: None,
            props: {
                "policy.expired": "1362471462",
            },
        },
    ],
}
"#
        );

        k9::snapshot!(
            ar.encode_value(),
            r#"
foo.example.net 1;\r
\tdkim/1=fail\r
\tpolicy.expired=1362471462
"#
        );
    }
}
