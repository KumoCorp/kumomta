use crate::headermap::EncodeHeaderValue;
use crate::nom_utils::{explain_nom, make_context_error, make_span, IResult, ParseError, Span};
use crate::{MailParsingError, Result, SharedString};
use bstr::{BStr, BString, ByteSlice, ByteVec};
use charset_normalizer_rs::Encoding;
use nom::branch::alt;
use nom::bytes::complete::{take_while, take_while1, take_while_m_n};
use nom::combinator::{all_consuming, map, opt, recognize};
use nom::error::context;
use nom::multi::{many0, many1, separated_list1};
use nom::sequence::{delimited, preceded, separated_pair, terminated};
use nom::{Compare, Input, Parser as _};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::marker::PhantomData;

impl MailParsingError {
    pub fn from_nom(input: Span, err: nom::Err<ParseError<Span<'_>>>) -> Self {
        MailParsingError::HeaderParse(explain_nom(input, err))
    }
}

/// Like nom::bytes::complete::tag, except that we print what the tag
/// was expecting if there was an error.
/// I feel like this should be the default behavior TBH.
fn tag<E>(tag: &'static str) -> TagParser<E> {
    TagParser {
        tag,
        e: PhantomData,
    }
}

/// Struct to support displaying better errors for tag()
struct TagParser<E> {
    tag: &'static str,
    e: PhantomData<E>,
}

/// All this fuss to show what we expected for the TagParser impl
impl<I, Error: nom::error::ParseError<I> + nom::error::FromExternalError<I, String>> nom::Parser<I>
    for TagParser<Error>
where
    I: Input + Compare<&'static str> + nom::AsBytes,
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

        match i.compare(self.tag) {
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

fn is_utf8_non_ascii(c: u8) -> bool {
    c == 0 || c >= 0x80
}

// ctl = { '\u{00}'..'\u{1f}' | "\u{7f}" }
fn is_ctl(c: u8) -> bool {
    match c {
        b'\x00'..=b'\x1f' | b'\x7f' => true,
        _ => false,
    }
}

fn not_angle(c: u8) -> bool {
    match c {
        b'<' | b'>' => false,
        _ => true,
    }
}

// char = { '\u{01}'..'\u{7f}' }
fn is_char(c: u8) -> bool {
    match c {
        0x01..=0xff => true,
        _ => false,
    }
}

fn is_especial(c: u8) -> bool {
    match c {
        b'(' | b')' | b'<' | b'>' | b'@' | b',' | b';' | b':' | b'/' | b'[' | b']' | b'?'
        | b'.' | b'=' => true,
        _ => false,
    }
}

fn is_token(c: u8) -> bool {
    is_char(c) && c != b' ' && !is_especial(c) && !is_ctl(c)
}

// vchar = { '\u{21}'..'\u{7e}' | utf8_non_ascii }
fn is_vchar(c: u8) -> bool {
    (0x21..=0x7e).contains(&c) || is_utf8_non_ascii(c)
}

fn is_atext(c: u8) -> bool {
    match c {
        b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'/' | b'=' | b'?'
        | b'^' | b'_' | b'`' | b'{' | b'|' | b'}' | b'~' => true,
        c => c.is_ascii_alphanumeric() || is_utf8_non_ascii(c),
    }
}

fn atext(input: Span) -> IResult<Span, Span> {
    context("atext", take_while1(is_atext)).parse(input)
}

fn is_obs_no_ws_ctl(c: u8) -> bool {
    match c {
        0x01..=0x08 | 0x0b..=0x0c | 0x0e..=0x1f | 0x7f => true,
        _ => false,
    }
}

fn is_obs_ctext(c: u8) -> bool {
    is_obs_no_ws_ctl(c)
}

// ctext = { '\u{21}'..'\u{27}' | '\u{2a}'..'\u{5b}' | '\u{5d}'..'\u{7e}' | obs_ctext | utf8_non_ascii }
fn is_ctext(c: u8) -> bool {
    match c {
        0x21..=0x27 | 0x2a..=0x5b | 0x5d..=0x7e => true,
        c => is_obs_ctext(c) || is_utf8_non_ascii(c),
    }
}

// dtext = { '\u{21}'..'\u{5a}' | '\u{5e}'..'\u{7e}' | obs_dtext | utf8_non_ascii }
// obs_dtext = { obs_no_ws_ctl | quoted_pair }
fn is_dtext(c: u8) -> bool {
    match c {
        0x21..=0x5a | 0x5e..=0x7e => true,
        c => is_obs_no_ws_ctl(c) || is_utf8_non_ascii(c),
    }
}

// qtext = { "\u{21}" | '\u{23}'..'\u{5b}' | '\u{5d}'..'\u{7e}' | obs_qtext | utf8_non_ascii }
// obs_qtext = { obs_no_ws_ctl }
fn is_qtext(c: u8) -> bool {
    match c {
        0x21 | 0x23..=0x5b | 0x5d..=0x7e => true,
        c => is_obs_no_ws_ctl(c) || is_utf8_non_ascii(c),
    }
}

fn is_tspecial(c: u8) -> bool {
    match c {
        b'(' | b')' | b'<' | b'>' | b'@' | b',' | b';' | b':' | b'\\' | b'"' | b'/' | b'['
        | b']' | b'?' | b'=' => true,
        _ => false,
    }
}

fn is_attribute_char(c: u8) -> bool {
    match c {
        b' ' | b'*' | b'\'' | b'%' => false,
        _ => is_char(c) && !is_ctl(c) && !is_tspecial(c),
    }
}

fn wsp(input: Span) -> IResult<Span, Span> {
    context("wsp", take_while1(|c| c == b' ' || c == b'\t')).parse(input)
}

fn newline(input: Span) -> IResult<Span, Span> {
    context("newline", recognize(preceded(opt(tag("\r")), tag("\n")))).parse(input)
}

// fws = { ((wsp* ~ "\r"? ~ "\n")* ~ wsp+) | obs_fws }
fn fws(input: Span) -> IResult<Span, Span> {
    context(
        "fws",
        alt((
            recognize(preceded(many0(preceded(many0(wsp), newline)), many1(wsp))),
            obs_fws,
        )),
    )
    .parse(input)
}

// obs_fws = { wsp+ ~ ("\r"? ~ "\n" ~ wsp+)* }
fn obs_fws(input: Span) -> IResult<Span, Span> {
    context(
        "obs_fws",
        recognize(preceded(many1(wsp), preceded(newline, many1(wsp)))),
    )
    .parse(input)
}

// mailbox_list = { (mailbox ~ ("," ~ mailbox)*) | obs_mbox_list }
fn mailbox_list(input: Span) -> IResult<Span, MailboxList> {
    let (loc, mailboxes) = context(
        "mailbox_list",
        alt((separated_list1(tag(","), mailbox), obs_mbox_list)),
    )
    .parse(input)?;
    Ok((loc, MailboxList(mailboxes)))
}

// obs_mbox_list = {  ((cfws? ~ ",")* ~ mailbox ~ ("," ~ (mailbox | cfws))*)+ }
fn obs_mbox_list(input: Span) -> IResult<Span, Vec<Mailbox>> {
    let (loc, entries) = context(
        "obs_mbox_list",
        many1(preceded(
            many0(preceded(opt(cfws), tag(","))),
            (
                mailbox,
                many0(preceded(
                    tag(","),
                    alt((map(mailbox, Some), map(cfws, |_| None))),
                )),
            ),
        )),
    )
    .parse(input)?;

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
        let (loc, address) = context("mailbox", addr_spec).parse(input)?;
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
            map(separated_list1(tag(","), address), AddressList),
            obs_address_list,
        )),
    )
    .parse(input)
}

// obs_addr_list = {  ((cfws? ~ ",")* ~ address ~ ("," ~ (address | cfws))*)+ }
fn obs_address_list(input: Span) -> IResult<Span, AddressList> {
    let (loc, entries) = context(
        "obs_address_list",
        many1(preceded(
            many0(preceded(opt(cfws), tag(","))),
            (
                address,
                many0(preceded(
                    tag(","),
                    alt((map(address, Some), map(cfws, |_| None))),
                )),
            ),
        )),
    )
    .parse(input)?;

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
    context("address", alt((map(mailbox, Address::Mailbox), group))).parse(input)
}

// group = { display_name ~ ":" ~ group_list? ~ ";" ~ cfws? }
fn group(input: Span) -> IResult<Span, Address> {
    let (loc, (name, _, group_list, _)) = context(
        "group",
        terminated(
            (display_name, tag(":"), opt(group_list), tag(";")),
            opt(cfws),
        ),
    )
    .parse(input)?;
    Ok((
        loc,
        Address::Group {
            name: name.into(),
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
    )
    .parse(input)
}

// obs_group_list = @{ (cfws? ~ ",")+ ~ cfws? }
fn obs_group_list(input: Span) -> IResult<Span, MailboxList> {
    context(
        "obs_group_list",
        map(
            terminated(many1(preceded(opt(cfws), tag(","))), opt(cfws)),
            |_| MailboxList(vec![]),
        ),
    )
    .parse(input)
}

// name_addr = { display_name? ~ angle_addr }
fn name_addr(input: Span) -> IResult<Span, Mailbox> {
    context(
        "name_addr",
        map((opt(display_name), angle_addr), |(name, address)| Mailbox {
            name: name.map(Into::into),
            address,
        }),
    )
    .parse(input)
}

// display_name = { phrase }
fn display_name(input: Span) -> IResult<Span, BString> {
    context("display_name", phrase).parse(input)
}

// phrase = { (encoded_word | word)+ | obs_phrase }
// obs_phrase = { (encoded_word | word) ~ (encoded_word | word | dot | cfws)* }
fn phrase(input: Span) -> IResult<Span, BString> {
    let (loc, (a, b)): (Span, (BString, Vec<Option<BString>>)) = context(
        "phrase",
        (
            alt((encoded_word, word)),
            many0(alt((
                map(cfws, |_| None),
                map(encoded_word, Option::Some),
                map(word, Option::Some),
                map(tag("."), |_dot| Some(BString::from("."))),
            ))),
        ),
    )
    .parse(input)?;
    let mut result = a;
    for item in b {
        if let Some(item) = item {
            result.push(b' ');
            result.push_str(item);
        }
    }
    Ok((loc, result))
}

// angle_addr = { cfws? ~ "<" ~ addr_spec ~ ">" ~ cfws? | obs_angle_addr }
fn angle_addr(input: Span) -> IResult<Span, AddrSpec> {
    context(
        "angle_addr",
        alt((
            delimited(
                opt(cfws),
                delimited(tag("<"), addr_spec, tag(">")),
                opt(cfws),
            ),
            obs_angle_addr,
        )),
    )
    .parse(input)
}

// obs_angle_addr = { cfws? ~ "<" ~ obs_route ~ addr_spec ~ ">" ~ cfws? }
fn obs_angle_addr(input: Span) -> IResult<Span, AddrSpec> {
    context(
        "obs_angle_addr",
        delimited(
            opt(cfws),
            delimited(tag("<"), preceded(obs_route, addr_spec), tag(">")),
            opt(cfws),
        ),
    )
    .parse(input)
}

// obs_route = { obs_domain_list ~ ":" }
// obs_domain_list = { (cfws | ",")* ~ "@" ~ domain ~ ("," ~ cfws? ~ ("@" ~ domain)?)* }
fn obs_route(input: Span) -> IResult<Span, Span> {
    context(
        "obs_route",
        recognize(terminated(
            (
                many0(alt((cfws, recognize(tag(","))))),
                recognize(tag("@")),
                recognize(domain),
                many0((tag(","), opt(cfws), opt((tag("@"), domain)))),
            ),
            tag(":"),
        )),
    )
    .parse(input)
}

// addr_spec = { local_part ~ "@" ~ domain }
fn addr_spec(input: Span) -> IResult<Span, AddrSpec> {
    let (loc, (local_part, domain)) =
        context("addr_spec", separated_pair(local_part, tag("@"), domain)).parse(input)?;
    Ok((
        loc,
        AddrSpec {
            local_part: local_part.into(),
            domain: domain.into(),
        },
    ))
}

fn parse_with<'a, R, F>(text: &'a [u8], parser: F) -> Result<R>
where
    F: Fn(Span<'a>) -> IResult<'a, Span<'a>, R>,
{
    let input = make_span(text);
    let (_, result) = all_consuming(parser)
        .parse(input)
        .map_err(|err| MailParsingError::from_nom(input, err))?;
    Ok(result)
}

#[cfg(test)]
#[test]
fn test_addr_spec() {
    k9::snapshot!(
        parse_with("darth.vader@a.galaxy.far.far.away".as_bytes(), addr_spec),
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
        parse_with(
            "\"darth.vader\"@a.galaxy.far.far.away".as_bytes(),
            addr_spec
        ),
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
        parse_with(
            "\"darth\".vader@a.galaxy.far.far.away".as_bytes(),
            addr_spec
        ),
        r#"
Err(
    HeaderParse(
        "Error at line 1, expected "@" but found ".":
"darth".vader@a.galaxy.far.far.away
       ^___________________________

while parsing addr_spec
",
    ),
)
"#
    );

    k9::snapshot!(
        parse_with("a@[127.0.0.1]".as_bytes(), addr_spec),
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
        parse_with("a@[IPv6::1]".as_bytes(), addr_spec),
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
fn atom(input: Span) -> IResult<Span, BString> {
    let (loc, text) = context("atom", delimited(opt(cfws), atext, opt(cfws))).parse(input)?;
    Ok((loc, (*text).into()))
}

// word = { atom | quoted_string }
fn word(input: Span) -> IResult<Span, BString> {
    context("word", alt((atom, quoted_string))).parse(input)
}

// obs_local_part = { word ~ (dot ~ word)* }
fn obs_local_part(input: Span) -> IResult<Span, BString> {
    let (loc, (word, dotted_words)) =
        context("obs_local_part", (word, many0((tag("."), word)))).parse(input)?;
    let mut result = word;

    for (_dot, w) in dotted_words {
        result.push(b'.');
        result.push_str(&w);
    }

    Ok((loc, result))
}

// local_part = { dot_atom | quoted_string | obs_local_part }
fn local_part(input: Span) -> IResult<Span, BString> {
    context("local_part", alt((dot_atom, quoted_string, obs_local_part))).parse(input)
}

// domain = { dot_atom | domain_literal | obs_domain }
fn domain(input: Span) -> IResult<Span, BString> {
    context("domain", alt((dot_atom, domain_literal, obs_domain))).parse(input)
}

// obs_domain = { atom ~ ( dot ~ atom)* }
fn obs_domain(input: Span) -> IResult<Span, BString> {
    let (loc, (atom, dotted_atoms)) =
        context("obs_domain", (atom, many0((tag("."), atom)))).parse(input)?;
    let mut result = atom;

    for (_dot, w) in dotted_atoms {
        result.push(b'.');
        result.push_str(&w);
    }

    Ok((loc, result))
}

// domain_literal = { cfws? ~ "[" ~ (fws? ~ dtext)* ~ fws? ~ "]" ~ cfws? }
fn domain_literal(input: Span) -> IResult<Span, BString> {
    let (loc, (bits, trailer)) = context(
        "domain_literal",
        delimited(
            opt(cfws),
            delimited(
                tag("["),
                (
                    many0((opt(fws), alt((take_while_m_n(1, 1, is_dtext), quoted_pair)))),
                    opt(fws),
                ),
                tag("]"),
            ),
            opt(cfws),
        ),
    )
    .parse(input)?;

    let mut result = BString::default();
    result.push(b'[');
    for (a, b) in bits {
        if let Some(a) = a {
            result.push_str(&a);
        }
        result.push_str(b);
    }
    if let Some(t) = trailer {
        result.push_str(&t);
    }
    result.push(b']');
    Ok((loc, result))
}

// dot_atom_text = @{ atext ~ ("." ~ atext)* }
fn dot_atom_text(input: Span) -> IResult<Span, BString> {
    let (loc, (a, b)) =
        context("dot_atom_text", (atext, many0(preceded(tag("."), atext)))).parse(input)?;
    let mut result: BString = (*a).into();
    for item in b {
        result.push(b'.');
        result.push_str(&item);
    }

    Ok((loc, result))
}

// dot_atom = { cfws? ~ dot_atom_text ~ cfws? }
fn dot_atom(input: Span) -> IResult<Span, BString> {
    context("dot_atom", delimited(opt(cfws), dot_atom_text, opt(cfws))).parse(input)
}

#[cfg(test)]
#[test]
fn test_dot_atom() {
    k9::snapshot!(
        parse_with("hello".as_bytes(), dot_atom),
        r#"
Ok(
    "hello",
)
"#
    );

    k9::snapshot!(
        parse_with("hello.there".as_bytes(), dot_atom),
        r#"
Ok(
    "hello.there",
)
"#
    );

    k9::snapshot!(
        parse_with("hello.".as_bytes(), dot_atom),
        r#"
Err(
    HeaderParse(
        "Error at line 1, in Eof:
hello.
     ^

",
    ),
)
"#
    );

    k9::snapshot!(
        parse_with("(wat)hello".as_bytes(), dot_atom),
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
            recognize((many1((opt(fws), comment)), opt(fws))),
            fws,
        ))),
    )
    .parse(input)
}

// comment = { "(" ~ (fws? ~ ccontent)* ~ fws? ~ ")" }
fn comment(input: Span) -> IResult<Span, Span> {
    context(
        "comment",
        recognize((tag("("), many0((opt(fws), ccontent)), opt(fws), tag(")"))),
    )
    .parse(input)
}

#[cfg(test)]
#[test]
fn test_comment() {
    k9::snapshot!(
        BStr::new(&parse_with("(wat)".as_bytes(), comment).unwrap()),
        "(wat)"
    );
}

// ccontent = { ctext | quoted_pair | comment | encoded_word }
fn ccontent(input: Span) -> IResult<Span, Span> {
    context(
        "ccontent",
        recognize(alt((
            recognize(take_while_m_n(1, 1, is_ctext)),
            recognize(quoted_pair),
            comment,
            recognize(encoded_word),
        ))),
    )
    .parse(input)
}

fn is_quoted_pair(c: u8) -> bool {
    match c {
        0x00 | b'\r' | b'\n' | b' ' => true,
        c => is_obs_no_ws_ctl(c) || is_vchar(c),
    }
}

// quoted_pair = { ( "\\"  ~ (vchar | wsp)) | obs_qp }
// obs_qp = { "\\" ~ ( "\u{00}" | obs_no_ws_ctl | "\r" | "\n") }
fn quoted_pair(input: Span) -> IResult<Span, Span> {
    context(
        "quoted_pair",
        preceded(tag("\\"), take_while_m_n(1, 1, is_quoted_pair)),
    )
    .parse(input)
}

// encoded_word = { "=?" ~ charset ~ ("*" ~ language)? ~ "?" ~ encoding ~ "?" ~ encoded_text ~ "?=" }
fn encoded_word(input: Span) -> IResult<Span, BString> {
    let (loc, (charset, _language, _, encoding, _, text)) = context(
        "encoded_word",
        delimited(
            tag("=?"),
            (
                charset,
                opt(preceded(tag("*"), language)),
                tag("?"),
                encoding,
                tag("?"),
                encoded_text,
            ),
            tag("?="),
        ),
    )
    .parse(input)?;

    let bytes = match *encoding.fragment() {
        b"B" | b"b" => data_encoding::BASE64_MIME
            .decode(text.as_bytes())
            .map_err(|err| {
                make_context_error(
                    input,
                    format!("encoded_word: base64 decode failed: {err:#}"),
                )
            })?,
        b"Q" | b"q" => {
            // for rfc2047 header encoding, _ can be used to represent a space
            let munged = text.replace("_", " ");
            // The quoted_printable crate will unhelpfully strip trailing space
            // from the decoded input string, and we must track and restore it
            let had_trailing_space = munged.ends_with_str(" ");
            let mut decoded = quoted_printable::decode(munged, quoted_printable::ParseMode::Robust)
                .map_err(|err| {
                    make_context_error(
                        input,
                        format!("encoded_word: quoted printable decode failed: {err:#}"),
                    )
                })?;
            if had_trailing_space && !decoded.ends_with(b" ") {
                decoded.push(b' ');
            }
            decoded
        }
        encoding => {
            let encoding = BStr::new(encoding);
            return Err(make_context_error(
                input,
                format!(
                    "encoded_word: invalid encoding '{encoding}', expected one of b, B, q or Q"
                ),
            ));
        }
    };

    let charset_name = charset.to_str().map_err(|err| {
        make_context_error(
            input,
            format!(
                "encoded_word: charset {} is not UTF-8: {err}",
                BStr::new(*charset)
            ),
        )
    })?;

    let charset = Encoding::by_name(&*charset_name).ok_or_else(|| {
        make_context_error(
            input,
            format!("encoded_word: unsupported charset '{charset_name}'"),
        )
    })?;

    let decoded = charset.decode_simple(&bytes).map_err(|err| {
        make_context_error(
            input,
            format!("encoded_word: failed to decode as '{charset_name}': {err}"),
        )
    })?;

    Ok((loc, decoded.into()))
}

// charset = @{ (!"*" ~ token)+ }
fn charset(input: Span) -> IResult<Span, Span> {
    context("charset", take_while1(|c| c != b'*' && is_token(c))).parse(input)
}

// language = @{ token+ }
fn language(input: Span) -> IResult<Span, Span> {
    context("language", take_while1(|c| c != b'*' && is_token(c))).parse(input)
}

// encoding = @{ token+ }
fn encoding(input: Span) -> IResult<Span, Span> {
    context("encoding", take_while1(|c| c != b'*' && is_token(c))).parse(input)
}

// encoded_text = @{ (!( " " | "?") ~ vchar)+ }
fn encoded_text(input: Span) -> IResult<Span, Span> {
    context(
        "encoded_text",
        take_while1(|c| is_vchar(c) && c != b' ' && c != b'?'),
    )
    .parse(input)
}

// quoted_string = { cfws? ~ "\"" ~ (fws? ~ qcontent)* ~ fws? ~ "\"" ~ cfws? }
fn quoted_string(input: Span) -> IResult<Span, BString> {
    let (loc, (bits, trailer)) = context(
        "quoted_string",
        delimited(
            opt(cfws),
            delimited(
                tag("\""),
                (many0((opt(fws), qcontent)), opt(fws)),
                tag("\""),
            ),
            opt(cfws),
        ),
    )
    .parse(input)?;

    let mut result = BString::default();
    for (a, b) in bits {
        if let Some(a) = a {
            result.push_str(&a);
        }
        result.push_str(b);
    }
    if let Some(t) = trailer {
        result.push_str(&t);
    }
    Ok((loc, result))
}

// qcontent = { qtext | quoted_pair }
fn qcontent(input: Span) -> IResult<Span, Span> {
    context(
        "qcontent",
        alt((take_while_m_n(1, 1, is_qtext), quoted_pair)),
    )
    .parse(input)
}

fn content_id(input: Span) -> IResult<Span, MessageID> {
    let (loc, id) = context("content_id", msg_id).parse(input)?;
    Ok((loc, id))
}

fn msg_id(input: Span) -> IResult<Span, MessageID> {
    let (loc, id) = context("msg_id", alt((strict_msg_id, relaxed_msg_id))).parse(input)?;
    Ok((loc, id))
}

fn relaxed_msg_id(input: Span) -> IResult<Span, MessageID> {
    let (loc, id) = context(
        "msg_id",
        delimited(
            preceded(opt(cfws), tag("<")),
            many0(take_while_m_n(1, 1, not_angle)),
            preceded(tag(">"), opt(cfws)),
        ),
    )
    .parse(input)?;

    let mut result = BString::default();
    for item in id.into_iter() {
        result.push_str(*item);
    }

    Ok((loc, MessageID(result)))
}

// msg_id_list = { msg_id+ }
fn msg_id_list(input: Span) -> IResult<Span, Vec<MessageID>> {
    context("msg_id_list", many1(msg_id)).parse(input)
}

// id_left = { dot_atom_text | obs_id_left }
// obs_id_left = { local_part }
fn id_left(input: Span) -> IResult<Span, BString> {
    context("id_left", alt((dot_atom_text, local_part))).parse(input)
}

// id_right = { dot_atom_text | no_fold_literal | obs_id_right }
// obs_id_right = { domain }
fn id_right(input: Span) -> IResult<Span, BString> {
    context("id_right", alt((dot_atom_text, no_fold_literal, domain))).parse(input)
}

// no_fold_literal = { "[" ~ dtext* ~ "]" }
fn no_fold_literal(input: Span) -> IResult<Span, BString> {
    context(
        "no_fold_literal",
        map(
            recognize((tag("["), take_while(is_dtext), tag("]"))),
            |s: Span| (*s).into(),
        ),
    )
    .parse(input)
}

// msg_id = { cfws? ~ "<" ~ id_left ~ "@" ~ id_right ~ ">" ~ cfws? }
fn strict_msg_id(input: Span) -> IResult<Span, MessageID> {
    let (loc, (left, _, right)) = context(
        "msg_id",
        delimited(
            preceded(opt(cfws), tag("<")),
            (id_left, tag("@"), id_right),
            preceded(tag(">"), opt(cfws)),
        ),
    )
    .parse(input)?;

    let mut result: BString = left.into();
    result.push_char('@');
    result.push_str(right);

    Ok((loc, MessageID(result)))
}

// obs_unstruct = { (( "\r"* ~ "\n"* ~ ((encoded_word | obs_utext)~ "\r"* ~ "\n"*)+) | fws)+ }
fn unstructured(input: Span) -> IResult<Span, BString> {
    #[derive(Debug)]
    enum Word {
        Encoded(BString),
        UText(BString),
        Fws,
    }

    let (loc, words) = context(
        "unstructured",
        many0(alt((
            preceded(
                map(take_while(|c| c == b'\r' || c == b'\n'), |_| Word::Fws),
                terminated(
                    alt((
                        map(encoded_word, Word::Encoded),
                        map(obs_utext, |s| Word::UText((*s).into())),
                    )),
                    map(take_while(|c| c == b'\r' || c == b'\n'), |_| Word::Fws),
                ),
            ),
            map(fws, |_| Word::Fws),
        ))),
    )
    .parse(input)?;

    #[derive(Debug)]
    enum ProcessedWord {
        Encoded(BString),
        Text(BString),
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
                Some(ProcessedWord::Text(prior)) => prior.push_str(c),
                _ => processed.push(ProcessedWord::Text(c)),
            },
        }
    }

    let mut result = BString::default();
    for word in processed {
        match word {
            ProcessedWord::Encoded(s) | ProcessedWord::Text(s) => {
                result.push_str(&s);
            }
            ProcessedWord::Fws => {
                result.push(b' ');
            }
        }
    }

    Ok((loc, result))
}

fn arc_authentication_results(input: Span) -> IResult<Span, ARCAuthenticationResults> {
    context(
        "arc_authentication_results",
        map(
            (
                preceded(opt(cfws), tag("i")),
                preceded(opt(cfws), tag("=")),
                preceded(opt(cfws), nom::character::complete::u8),
                preceded(opt(cfws), tag(";")),
                preceded(opt(cfws), value),
                opt(preceded(cfws, nom::character::complete::u32)),
                alt((no_result, many1(resinfo))),
                opt(cfws),
            ),
            |(_i, _eq, instance, _semic, serv_id, version, results, _)| ARCAuthenticationResults {
                instance,
                serv_id: serv_id.into(),
                version,
                results,
            },
        ),
    )
    .parse(input)
}

fn authentication_results(input: Span) -> IResult<Span, AuthenticationResults> {
    context(
        "authentication_results",
        map(
            (
                preceded(opt(cfws), value),
                opt(preceded(cfws, nom::character::complete::u32)),
                alt((no_result, many1(resinfo))),
                opt(cfws),
            ),
            |(serv_id, version, results, _)| AuthenticationResults {
                serv_id: serv_id.into(),
                version,
                results,
            },
        ),
    )
    .parse(input)
}

fn no_result(input: Span) -> IResult<Span, Vec<AuthenticationResult>> {
    context(
        "no_result",
        map((opt(cfws), tag(";"), opt(cfws), tag("none")), |_| vec![]),
    )
    .parse(input)
}

fn resinfo(input: Span) -> IResult<Span, AuthenticationResult> {
    context(
        "resinfo",
        map(
            (
                opt(cfws),
                tag(";"),
                methodspec,
                opt(preceded(cfws, reasonspec)),
                opt(many1(propspec)),
            ),
            |(_, _, (method, method_version, result), reason, props)| AuthenticationResult {
                method: method.into(),
                method_version,
                result: result.into(),
                reason: reason.map(Into::into),
                props: match props {
                    None => BTreeMap::default(),
                    Some(props) => props.into_iter().collect(),
                },
            },
        ),
    )
    .parse(input)
}

fn methodspec(input: Span) -> IResult<Span, (BString, Option<u32>, BString)> {
    context(
        "methodspec",
        map(
            (
                opt(cfws),
                (keyword, opt(methodversion)),
                opt(cfws),
                tag("="),
                opt(cfws),
                keyword,
            ),
            |(_, (method, methodversion), _, _, _, result)| (method, methodversion, result),
        ),
    )
    .parse(input)
}

// Taken from https://datatracker.ietf.org/doc/html/rfc8601 which says
// that this is the same as the SMTP Keyword token
fn keyword(input: Span) -> IResult<Span, BString> {
    context(
        "keyword",
        map(
            take_while1(|c: u8| c.is_ascii_alphanumeric() || c == b'+' || c == b'-'),
            |s: Span| (*s).into(),
        ),
    )
    .parse(input)
}

fn methodversion(input: Span) -> IResult<Span, u32> {
    context(
        "methodversion",
        preceded(
            (opt(cfws), tag("/"), opt(cfws)),
            nom::character::complete::u32,
        ),
    )
    .parse(input)
}

fn reasonspec(input: Span) -> IResult<Span, BString> {
    context(
        "reason",
        map(
            (tag("reason"), opt(cfws), tag("="), opt(cfws), value),
            |(_, _, _, _, value)| value,
        ),
    )
    .parse(input)
}

fn propspec(input: Span) -> IResult<Span, (BString, BString)> {
    context(
        "propspec",
        map(
            (
                opt(cfws),
                keyword,
                opt(cfws),
                tag("."),
                opt(cfws),
                keyword,
                opt(cfws),
                tag("="),
                opt(cfws),
                alt((
                    map(preceded(tag("@"), domain), |d| {
                        let mut at_dom = BString::from("@");
                        at_dom.push_str(d);
                        at_dom
                    }),
                    map(separated_pair(local_part, tag("@"), domain), |(u, d)| {
                        let mut result: BString = u.into();
                        result.push(b'@');
                        result.push_str(d);
                        result
                    }),
                    domain,
                    // value must be last in this alternation
                    value,
                )),
                opt(cfws),
            ),
            |(_, ptype, _, _, _, property, _, _, _, value, _)| {
                (format!("{ptype}.{property}").into(), value)
            },
        ),
    )
    .parse(input)
}

// obs_utext = @{ "\u{00}" | obs_no_ws_ctl | vchar }
fn obs_utext(input: Span) -> IResult<Span, Span> {
    context(
        "obs_utext",
        take_while_m_n(1, 1, |c| c == 0x00 || is_obs_no_ws_ctl(c) || is_vchar(c)),
    )
    .parse(input)
}

fn is_mime_token(c: u8) -> bool {
    is_char(c) && c != b' ' && !is_ctl(c) && !is_tspecial(c)
}

// mime_token = { (!(" " | ctl | tspecials) ~ char)+ }
fn mime_token(input: Span) -> IResult<Span, Span> {
    context("mime_token", take_while1(is_mime_token)).parse(input)
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
            (
                mime_token,
                opt(cfws),
                tag("/"),
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
                    preceded(opt(tag(";")), opt(cfws)),
                    terminated(parameter, opt(cfws)),
                )),
            ),
        ),
    )
    .parse(input)?;

    let mut value: BString = (*mime_type).into();
    value.push_char('/');
    value.push_str(mime_subtype);

    Ok((loc, MimeParameters { value, parameters }))
}

fn content_transfer_encoding(input: Span) -> IResult<Span, MimeParameters> {
    let (loc, (value, _, parameters)) = context(
        "content_transfer_encoding",
        preceded(
            opt(cfws),
            (
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
                    preceded(opt(tag(";")), opt(cfws)),
                    terminated(parameter, opt(cfws)),
                )),
            ),
        ),
    )
    .parse(input)?;

    Ok((
        loc,
        MimeParameters {
            value: value.as_bytes().into(),
            parameters,
        },
    ))
}

// parameter = { regular_parameter | extended_parameter }
fn parameter(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "parameter",
        alt((
            // Note that RFC2047 explicitly prohibits both of
            // these 2047 cases from appearing here, but that
            // major MUAs produce this sort of prohibited content
            // and we thus need to accommodate it
            param_with_unquoted_rfc2047,
            param_with_quoted_rfc2047,
            regular_parameter,
            extended_param_with_charset,
            extended_param_no_charset,
        )),
    )
    .parse(input)
}

fn param_with_unquoted_rfc2047(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "param_with_unquoted_rfc2047",
        map(
            (attribute, opt(cfws), tag("="), opt(cfws), encoded_word),
            |(name, _, _, _, value)| MimeParameter {
                name: name.as_bytes().into(),
                value: value.as_bytes().into(),
                section: None,
                encoding: MimeParameterEncoding::UnquotedRfc2047,
                mime_charset: None,
                mime_language: None,
            },
        ),
    )
    .parse(input)
}

fn param_with_quoted_rfc2047(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "param_with_quoted_rfc2047",
        map(
            (
                attribute,
                opt(cfws),
                tag("="),
                opt(cfws),
                delimited(tag("\""), encoded_word, tag("\"")),
            ),
            |(name, _, _, _, value)| MimeParameter {
                name: name.as_bytes().into(),
                value: value.as_bytes().into(),
                section: None,
                encoding: MimeParameterEncoding::QuotedRfc2047,
                mime_charset: None,
                mime_language: None,
            },
        ),
    )
    .parse(input)
}

fn extended_param_with_charset(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "extended_param_with_charset",
        map(
            (
                attribute,
                opt(section),
                tag("*"),
                opt(cfws),
                tag("="),
                opt(cfws),
                opt(mime_charset),
                tag("'"),
                opt(mime_language),
                tag("'"),
                map(
                    recognize(many0(alt((ext_octet, take_while1(is_attribute_char))))),
                    |s: Span| (*s).into(),
                ),
            ),
            |(name, section, _, _, _, _, mime_charset, _, mime_language, _, value)| MimeParameter {
                name: name.as_bytes().into(),
                section,
                mime_charset: mime_charset.map(|s| s.as_bytes().into()),
                mime_language: mime_language.map(|s| s.as_bytes().into()),
                encoding: MimeParameterEncoding::Rfc2231,
                value,
            },
        ),
    )
    .parse(input)
}

fn extended_param_no_charset(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "extended_param_no_charset",
        map(
            (
                attribute,
                opt(section),
                opt(tag("*")),
                opt(cfws),
                tag("="),
                opt(cfws),
                alt((
                    quoted_string,
                    map(
                        recognize(many0(alt((ext_octet, take_while1(is_attribute_char))))),
                        |s: Span| (*s).into(),
                    ),
                )),
            ),
            |(name, section, star, _, _, _, value)| MimeParameter {
                name: name.as_bytes().into(),
                section,
                mime_charset: None,
                mime_language: None,
                encoding: if star.is_some() {
                    MimeParameterEncoding::Rfc2231
                } else {
                    MimeParameterEncoding::None
                },
                value,
            },
        ),
    )
    .parse(input)
}

fn mime_charset(input: Span) -> IResult<Span, Span> {
    context(
        "mime_charset",
        take_while1(|c| is_mime_token(c) && c != b'\''),
    )
    .parse(input)
}

fn mime_language(input: Span) -> IResult<Span, Span> {
    context(
        "mime_language",
        take_while1(|c| is_mime_token(c) && c != b'\''),
    )
    .parse(input)
}

fn ext_octet(input: Span) -> IResult<Span, Span> {
    context(
        "ext_octet",
        recognize((
            tag("%"),
            take_while_m_n(2, 2, |b: u8| b.is_ascii_hexdigit()),
        )),
    )
    .parse(input)
}

// section = { "*" ~ ASCII_DIGIT+ }
fn section(input: Span) -> IResult<Span, u32> {
    context("section", preceded(tag("*"), nom::character::complete::u32)).parse(input)
}

// regular_parameter = { attribute ~ cfws? ~ "=" ~ cfws? ~ value }
fn regular_parameter(input: Span) -> IResult<Span, MimeParameter> {
    context(
        "regular_parameter",
        map(
            (attribute, opt(cfws), tag("="), opt(cfws), value),
            |(name, _, _, _, value)| MimeParameter {
                name: name.as_bytes().into(),
                value: value.as_bytes().into(),
                section: None,
                encoding: MimeParameterEncoding::None,
                mime_charset: None,
                mime_language: None,
            },
        ),
    )
    .parse(input)
}

// attribute = { attribute_char+ }
// attribute_char = { !(" " | ctl | tspecials | "*" | "'" | "%") ~ char }
fn attribute(input: Span) -> IResult<Span, Span> {
    context("attribute", take_while1(is_attribute_char)).parse(input)
}

fn value(input: Span) -> IResult<Span, BString> {
    context(
        "value",
        alt((map(mime_token, |s: Span| (*s).into()), quoted_string)),
    )
    .parse(input)
}

pub struct Parser;

impl Parser {
    pub fn parse_mailbox_list_header(text: &[u8]) -> Result<MailboxList> {
        parse_with(text, mailbox_list)
    }

    pub fn parse_mailbox_header(text: &[u8]) -> Result<Mailbox> {
        parse_with(text, mailbox)
    }

    pub fn parse_address_list_header(text: &[u8]) -> Result<AddressList> {
        parse_with(text, address_list)
    }

    pub fn parse_msg_id_header(text: &[u8]) -> Result<MessageID> {
        parse_with(text, msg_id)
    }

    pub fn parse_msg_id_header_list(text: &[u8]) -> Result<Vec<MessageID>> {
        parse_with(text, msg_id_list)
    }

    pub fn parse_content_id_header(text: &[u8]) -> Result<MessageID> {
        parse_with(text, content_id)
    }

    pub fn parse_content_type_header(text: &[u8]) -> Result<MimeParameters> {
        parse_with(text, content_type)
    }

    pub fn parse_content_transfer_encoding_header(text: &[u8]) -> Result<MimeParameters> {
        parse_with(text, content_transfer_encoding)
    }

    pub fn parse_unstructured_header(text: &[u8]) -> Result<BString> {
        parse_with(text, unstructured)
    }

    pub fn parse_authentication_results_header(text: &[u8]) -> Result<AuthenticationResults> {
        parse_with(text, authentication_results)
    }

    pub fn parse_arc_authentication_results_header(
        text: &[u8],
    ) -> Result<ARCAuthenticationResults> {
        parse_with(text, arc_authentication_results)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ARCAuthenticationResults {
    pub instance: u8,
    pub serv_id: BString,
    pub version: Option<u32>,
    pub results: Vec<AuthenticationResult>,
}

impl EncodeHeaderValue for ARCAuthenticationResults {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = format!("i={}; ", self.instance).into_bytes();

        match self.version {
            Some(v) => result.push_str(&format!("{} {v}", self.serv_id)),
            None => result.push_str(&self.serv_id),
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
                result.push(b'=');
                emit_value_token(res.result.as_bytes(), &mut result);
                if let Some(reason) = &res.reason {
                    result.push_str(" reason=");
                    emit_value_token(reason.as_bytes(), &mut result);
                }
                for (k, v) in &res.props {
                    result.push_str(&format!("\r\n\t{k}="));
                    emit_value_token(v.as_bytes(), &mut result);
                }
            }
        }

        result.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticationResults {
    pub serv_id: BString,
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub results: Vec<AuthenticationResult>,
}

/// Emits a value that was parsed by `value`, into target
fn emit_value_token(value: &[u8], target: &mut Vec<u8>) {
    let use_quoted_string = !value.iter().all(|&c| is_mime_token(c) || c == b'@');
    if use_quoted_string {
        target.push(b'"');
        for (start, end, c) in value.char_indices() {
            if c == '"' || c == '\\' {
                target.push(b'\\');
            }
            target.push_str(&value[start..end]);
        }
        target.push(b'"');
    } else {
        target.push_str(value);
    }
}

impl EncodeHeaderValue for AuthenticationResults {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = match self.version {
            Some(v) => format!("{} {v}", self.serv_id).into_bytes(),
            None => self.serv_id.to_vec(),
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
                result.push(b'=');
                emit_value_token(res.result.as_bytes(), &mut result);
                if let Some(reason) = &res.reason {
                    result.push_str(" reason=");
                    emit_value_token(reason.as_bytes(), &mut result);
                }
                for (k, v) in &res.props {
                    result.push_str(&format!("\r\n\t{k}="));
                    emit_value_token(v.as_bytes(), &mut result);
                }
            }
        }

        result.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticationResult {
    pub method: BString,
    #[serde(default)]
    pub method_version: Option<u32>,
    pub result: BString,
    #[serde(default)]
    pub reason: Option<BString>,
    #[serde(default)]
    pub props: BTreeMap<BString, BString>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddrSpec {
    pub local_part: BString,
    pub domain: BString,
}

impl AddrSpec {
    pub fn new(local_part: &str, domain: &str) -> Self {
        Self {
            local_part: local_part.into(),
            domain: domain.into(),
        }
    }

    pub fn parse(email: &str) -> Result<Self> {
        parse_with(email.as_bytes(), addr_spec)
    }
}

impl EncodeHeaderValue for AddrSpec {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result: Vec<u8> = vec![];

        let needs_quoting = !self.local_part.iter().all(|&c| is_atext(c) || c == b'.');
        if needs_quoting {
            result.push(b'"');
            // RFC5321 4.1.2 qtextSMTP:
            // within a quoted string, any ASCII graphic or space is permitted without
            // blackslash-quoting except double-quote and the backslash itself.

            for &c in self.local_part.iter() {
                if c == b'"' || c == b'\\' {
                    result.push(b'\\');
                }
                result.push(c);
            }
            result.push(b'"');
        } else {
            result.push_str(&self.local_part);
        }
        result.push(b'@');
        result.push_str(&self.domain);

        result.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Address {
    Mailbox(Mailbox),
    Group { name: BString, entries: MailboxList },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, transparent)]
pub struct AddressList(pub Vec<Address>);

impl std::ops::Deref for AddressList {
    type Target = Vec<Address>;
    fn deref(&self) -> &Vec<Address> {
        &self.0
    }
}

impl AddressList {
    pub fn extract_first_mailbox(&self) -> Option<&Mailbox> {
        let address = self.0.first()?;
        match address {
            Address::Mailbox(mailbox) => Some(mailbox),
            Address::Group { entries, .. } => entries.extract_first_mailbox(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, transparent)]
pub struct MailboxList(pub Vec<Mailbox>);

impl std::ops::Deref for MailboxList {
    type Target = Vec<Mailbox>;
    fn deref(&self) -> &Vec<Mailbox> {
        &self.0
    }
}

impl MailboxList {
    pub fn extract_first_mailbox(&self) -> Option<&Mailbox> {
        self.0.first()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mailbox {
    pub name: Option<BString>,
    pub address: AddrSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageID(pub BString);

impl EncodeHeaderValue for MessageID {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = Vec::<u8>::with_capacity(self.0.len() + 2);
        result.push(b'<');
        result.push_str(&self.0);
        result.push(b'>');
        result.into()
    }
}

impl EncodeHeaderValue for Vec<MessageID> {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = BString::default();
        for id in self {
            if !result.is_empty() {
                result.push_str("\r\n\t");
            }
            result.push(b'<');
            result.push_str(&id.0);
            result.push(b'>');
        }
        result.into()
    }
}

// In theory, everyone would be aware of RFC 2231 and we can stop here,
// but in practice, things are messy.  At some point someone started
// to emit encoded-words insides quoted-string values, and for the sake
// of compatibility what we see now is technically illegal stuff like
// Content-Disposition: attachment; filename="=?UTF-8?B?5pel5pys6Kqe44Gu5re75LuY?="
// being used to represent UTF-8 filenames.
// As such, in our RFC 2231 handling, we also need to accommodate
// these bogus representations, hence their presence in this enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MimeParameterEncoding {
    None,
    Rfc2231,
    UnquotedRfc2047,
    QuotedRfc2047,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MimeParameter {
    pub name: BString,
    pub section: Option<u32>,
    pub mime_charset: Option<BString>,
    pub mime_language: Option<BString>,
    pub encoding: MimeParameterEncoding,
    pub value: BString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MimeParameters {
    pub value: BString,
    parameters: Vec<MimeParameter>,
}

impl MimeParameters {
    pub fn new(value: impl AsRef<[u8]>) -> Self {
        Self {
            value: value.as_ref().into(),
            parameters: vec![],
        }
    }

    /// Decode all named parameters per RFC 2231 and return a map
    /// of the parameter names to parameters values.
    /// Incorrectly encoded parameters are silently ignored
    /// and are not returned in the resulting map.
    pub fn parameter_map(&self) -> BTreeMap<BString, BString> {
        let mut map = BTreeMap::new();

        fn contains_key_ignore_case(map: &BTreeMap<BString, BString>, key: &[u8]) -> bool {
            for k in map.keys() {
                if k.eq_ignore_ascii_case(key) {
                    return true;
                }
            }
            false
        }

        for entry in &self.parameters {
            let name = entry.name.as_bytes();
            if !contains_key_ignore_case(&map, name) {
                if let Some(value) = self.get(name) {
                    map.insert(name.into(), value);
                }
            }
        }

        map
    }

    /// Retrieve the value for a named parameter.
    /// This method will attempt to decode any %-encoded values
    /// per RFC 2231 and combine multi-element fields into a single
    /// contiguous value.
    /// Invalid charsets and encoding will be silently ignored.
    pub fn get(&self, name: impl AsRef<[u8]>) -> Option<BString> {
        let name = name.as_ref();
        let mut elements: Vec<_> = self
            .parameters
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(name.as_bytes()))
            .collect();
        if elements.is_empty() {
            return None;
        }
        elements.sort_by(|a, b| a.section.cmp(&b.section));

        let mut mime_charset = None;
        let mut result: Vec<u8> = vec![];

        for ele in elements {
            if let Some(cset) = ele.mime_charset.as_ref().and_then(|b| b.to_str().ok()) {
                mime_charset = Encoding::by_name(&*cset);
            }

            match ele.encoding {
                MimeParameterEncoding::Rfc2231 => {
                    if let Some(charset) = mime_charset.as_ref() {
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
                                                    value <<= 4;
                                                    value |= n as u32 as u8 - b'0';
                                                }
                                                'a'..='f' => {
                                                    value <<= 4;
                                                    value |= (n as u32 as u8 - b'a') + 10;
                                                }
                                                'A'..='F' => {
                                                    value <<= 4;
                                                    value |= (n as u32 as u8 - b'A') + 10;
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

                        if let Ok(decoded) = charset.decode_simple(&bytes) {
                            result.push_str(&decoded);
                        }
                    } else {
                        result.push_str(&ele.value);
                    }
                }
                MimeParameterEncoding::UnquotedRfc2047
                | MimeParameterEncoding::QuotedRfc2047
                | MimeParameterEncoding::None => {
                    result.push_str(&ele.value);
                }
            }
        }

        Some(result.into())
    }

    /// Remove the named parameter
    pub fn remove(&mut self, name: impl AsRef<[u8]>) {
        let name = name.as_ref();
        self.parameters
            .retain(|p| !p.name.eq_ignore_ascii_case(name));
    }

    pub fn set(&mut self, name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.set_with_encoding(name, value, MimeParameterEncoding::None)
    }

    pub(crate) fn set_with_encoding(
        &mut self,
        name: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        encoding: MimeParameterEncoding,
    ) {
        self.remove(name.as_ref());

        self.parameters.push(MimeParameter {
            name: name.as_ref().into(),
            value: value.as_ref().into(),
            section: None,
            mime_charset: None,
            mime_language: None,
            encoding,
        });
    }

    pub fn is_multipart(&self) -> bool {
        self.value.starts_with_str("message/") || self.value.starts_with_str("multipart/")
    }

    pub fn is_text(&self) -> bool {
        self.value.starts_with_str("text/")
    }
}

impl EncodeHeaderValue for MimeParameters {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result = self.value.clone();
        let names: BTreeMap<&BStr, MimeParameterEncoding> = self
            .parameters
            .iter()
            .map(|p| (p.name.as_bstr(), p.encoding))
            .collect();

        for (name, stated_encoding) in names {
            let value = self.get(name).expect("name to be present");

            match stated_encoding {
                MimeParameterEncoding::UnquotedRfc2047 => {
                    let encoded = qp_encode(&value);
                    result.push_str(&format!(";\r\n\t{name}={encoded}"));
                }
                MimeParameterEncoding::QuotedRfc2047 => {
                    let encoded = qp_encode(&value);
                    result.push_str(&format!(";\r\n\t{name}=\"{encoded}\""));
                }
                MimeParameterEncoding::None | MimeParameterEncoding::Rfc2231 => {
                    let needs_encoding = value.iter().any(|&c| !is_mime_token(c) || !c.is_ascii());
                    // Prefer to use quoted_string representation when possible, as it doesn't
                    // require any RFC 2231 encoding
                    let use_quoted_string = value
                        .iter()
                        .all(|&c| (is_qtext(c) || is_quoted_pair(c)) && c.is_ascii());

                    let mut params = vec![];
                    let mut chars = value.char_indices().peekable();
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

                        let mut encoded: Vec<u8> = vec![];

                        while encoded.len() < limit {
                            let Some((start, end, c)) = chars.next() else {
                                break;
                            };
                            let s = &value[start..end];

                            if use_quoted_string {
                                if c == '"' || c == '\\' {
                                    encoded.push(b'\\');
                                }
                                encoded.push_str(s);
                            } else if (c as u32) <= 0xff
                                && is_mime_token(c as u32 as u8)
                                && (!needs_encoding || c != '%')
                            {
                                encoded.push_str(s);
                            } else {
                                for b in s.bytes() {
                                    encoded.push(b'%');
                                    encoded.push(HEX_CHARS[(b as usize) >> 4]);
                                    encoded.push(HEX_CHARS[(b as usize) & 0x0f]);
                                }
                            }
                        }

                        if use_quoted_string {
                            encoded.push(b'"');
                        }

                        params.push(MimeParameter {
                            name: name.into(),
                            section: Some(count as u32),
                            mime_charset: if is_first { Some("UTF-8".into()) } else { None },
                            mime_language: None,
                            encoding: if needs_encoding {
                                MimeParameterEncoding::Rfc2231
                            } else {
                                MimeParameterEncoding::None
                            },
                            value: encoded.into(),
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
                            .unwrap_or_else(String::new);

                        let uses_encoding =
                            if !use_quoted_string && p.encoding == MimeParameterEncoding::Rfc2231 {
                                "*"
                            } else {
                                ""
                            };
                        let charset = if use_quoted_string {
                            BStr::new("\"")
                        } else {
                            p.mime_charset
                                .as_ref()
                                .map(|b| b.as_bstr())
                                .unwrap_or(BStr::new(""))
                        };
                        let lang = p
                            .mime_language
                            .as_ref()
                            .map(|b| b.as_bstr())
                            .unwrap_or(BStr::new(""));

                        let line = format!(
                            "{name}{section}{uses_encoding}={charset}{charset_tick}{lang}{lang_tick}{value}",
                            name = &p.name,
                            value = &p.value
                        );
                        result.push_str(&line);
                    }
                }
            }
        }
        result.into()
    }
}

static HEX_CHARS: &[u8] = b"0123456789ABCDEF";

pub(crate) fn qp_encode(s: &[u8]) -> String {
    let prefix = b"=?UTF-8?q?";
    let suffix = b"?=";
    let limit = 72 - (prefix.len() + suffix.len());

    let mut result = Vec::with_capacity(s.len());

    result.extend_from_slice(prefix);
    let mut line_length = 0;

    enum Bytes<'a> {
        Passthru(&'a [u8]),
        Encode(&'a [u8]),
    }

    // Iterate by char so that we don't confuse space (0x20) with a
    // utf8 subsequence and incorrectly encode the input string.
    for (start, end, c) in s.char_indices() {
        let bytes = &s[start..end];

        let b = if (c.is_ascii_alphanumeric() || c.is_ascii_punctuation())
            && c != '?'
            && c != '='
            && c != ' '
            && c != '\t'
        {
            Bytes::Passthru(bytes)
        } else if c == ' ' {
            Bytes::Passthru(b"_")
        } else {
            Bytes::Encode(bytes)
        };

        let need_len = match b {
            Bytes::Passthru(b) => b.len(),
            Bytes::Encode(b) => b.len() * 3,
        };

        if need_len > limit - line_length {
            // Need to wrap
            result.extend_from_slice(suffix);
            result.extend_from_slice(b"\r\n\t");
            result.extend_from_slice(prefix);
            line_length = 0;
        }

        match b {
            Bytes::Passthru(c) => {
                result.extend_from_slice(c);
            }
            Bytes::Encode(bytes) => {
                for &c in bytes {
                    result.push(b'=');
                    result.push(HEX_CHARS[(c as usize) >> 4]);
                    result.push(HEX_CHARS[(c as usize) & 0x0f]);
                }
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
        b"hello, I am a line that is this long, or maybe a little \
        bit longer than this, and that should get wrapped by the encoder",
    );
    k9::snapshot!(
        encoded,
        r#"
=?UTF-8?q?hello,_I_am_a_line_that_is_this_long,_or_maybe_a_little_bit_?=\r
\t=?UTF-8?q?longer_than_this,_and_that_should_get_wrapped_by_the_encoder?=
"#
    );
}

/// Quote input string `s`, using a backslash escape, if any
/// of the characters is NOT atext.  When quoting, the input
/// string is enclosed in quotes.
fn quote_string(s: impl AsRef<[u8]>) -> BString {
    let s = s.as_ref();

    if s.iter().any(|&c| !is_atext(c)) {
        let mut result = Vec::<u8>::with_capacity(s.len() + 4);
        result.push(b'"');
        for (start, end, c) in s.char_indices() {
            let c = c as u32;
            if c <= 0xff {
                let c = c as u8;
                if !c.is_ascii_whitespace() && !is_qtext(c) && !is_atext(c) {
                    result.push(b'\\');
                }
            }
            result.push_str(&s[start..end]);
        }
        result.push(b'"');
        result.into()
    } else {
        s.into()
    }
}

#[cfg(test)]
#[test]
fn test_quote_string() {
    k9::snapshot!(
        quote_string("TEST [ne_pas_repondre]"),
        r#""TEST [ne_pas_repondre]""#
    );
    k9::snapshot!(quote_string("hello"), "hello");
    k9::snapshot!(quote_string("hello there"), r#""hello there""#);
    k9::snapshot!(quote_string("hello, there"), "\"hello, there\"");
    k9::snapshot!(quote_string("hello \"there\""), r#""hello \\"there\\"""#);
    k9::snapshot!(
        quote_string("hello c:\\backslash"),
        r#""hello c:\\\\backslash""#
    );
    k9::assert_equal!(quote_string("hello\n there"), "\"hello\n there\"");
}

impl EncodeHeaderValue for Mailbox {
    fn encode_value(&self) -> SharedString<'static> {
        match &self.name {
            Some(name) => {
                let mut value: Vec<u8> = if name.is_ascii() {
                    quote_string(name).into()
                } else {
                    qp_encode(name.as_bytes()).into_bytes()
                };

                value.push_str(" <");
                value.push_str(self.address.encode_value().as_bytes());
                value.push(b'>');
                value.into()
            }
            None => {
                let mut result: Vec<u8> = vec![];
                result.push(b'<');
                result.push_str(self.address.encode_value().as_bytes());
                result.push(b'>');
                result.into()
            }
        }
    }
}

impl EncodeHeaderValue for MailboxList {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result: Vec<u8> = vec![];
        for mailbox in &self.0 {
            if !result.is_empty() {
                result.push_str(",\r\n\t");
            }
            result.push_str(mailbox.encode_value().as_bytes());
        }
        result.into()
    }
}

impl EncodeHeaderValue for Address {
    fn encode_value(&self) -> SharedString<'static> {
        match self {
            Self::Mailbox(mbox) => mbox.encode_value(),
            Self::Group { name, entries } => {
                let mut result: Vec<u8> = vec![];
                result.push_str(name);
                result.push(b':');
                result.push_str(entries.encode_value().as_bytes());
                result.push(b';');
                result.into()
            }
        }
    }
}

impl EncodeHeaderValue for AddressList {
    fn encode_value(&self) -> SharedString<'static> {
        let mut result: Vec<u8> = vec![];
        for address in &self.0 {
            if !result.is_empty() {
                result.push_str(",\r\n\t");
            }
            result.push_str(address.encode_value().as_bytes());
        }
        result.into()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Header, MessageConformance, MimePart};

    #[test]
    fn mailbox_encodes_at() {
        let mbox = Mailbox {
            name: Some("foo@bar.com".into()),
            address: AddrSpec {
                local_part: "foo".into(),
                domain: "bar.com".into(),
            },
        };
        assert_eq!(mbox.encode_value(), "\"foo@bar.com\" <foo@bar.com>");
    }

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
InvalidHeaderValueDuringGet {
    header_name: "Sender",
    error: HeaderParse(
        "Error at line 1, expected "@" but found ".":
hello..there@docomo.ne.jp
     ^___________________

while parsing addr_spec
while parsing mailbox
",
    ),
}
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
    fn attachment_filename_mess_totally_bogus() {
        let message = concat!("Content-Disposition: attachment; filename=@\n", "\n\n");
        let msg = MimePart::parse(message).unwrap();
        eprintln!("{msg:#?}");

        assert!(msg
            .conformance()
            .contains(MessageConformance::INVALID_MIME_HEADERS));
        msg.headers().content_disposition().unwrap_err();

        // There is no Content-Disposition in the rebuilt message, because
        // there was no valid Content-Disposition in what we parsed
        let rebuilt = msg.rebuild(None).unwrap();
        k9::assert_equal!(rebuilt.headers().content_disposition(), Ok(None));
    }

    #[test]
    fn attachment_filename_mess_aberrant() {
        let message = concat!(
            "Content-Disposition: attachment; filename= =?UTF-8?B?5pel5pys6Kqe44Gu5re75LuY?=\n",
            "\n\n"
        );
        let msg = MimePart::parse(message).unwrap();

        let cd = msg.headers().content_disposition().unwrap().unwrap();
        k9::assert_equal!(cd.get("filename").unwrap(), "日本語の添付");

        let encoded = cd.encode_value();
        k9::assert_equal!(encoded, "attachment;\r\n\tfilename==?UTF-8?q?=E6=97=A5=E6=9C=AC=E8=AA=9E=E3=81=AE=E6=B7=BB=E4=BB=98?=");
    }

    #[test]
    fn attachment_filename_mess_gmail() {
        let message = concat!(
            "Content-Disposition: attachment; filename=\"=?UTF-8?B?5pel5pys6Kqe44Gu5re75LuY?=\"\n",
            "Content-Type: text/plain;\n",
            "   name=\"=?UTF-8?B?5pel5pys6Kqe44Gu5re75LuY?=\"\n",
            "\n\n"
        );
        let msg = MimePart::parse(message).unwrap();

        let cd = msg.headers().content_disposition().unwrap().unwrap();
        k9::assert_equal!(cd.get("filename").unwrap(), "日本語の添付");
        let encoded = cd.encode_value();
        k9::assert_equal!(encoded, "attachment;\r\n\tfilename=\"=?UTF-8?q?=E6=97=A5=E6=9C=AC=E8=AA=9E=E3=81=AE=E6=B7=BB=E4=BB=98?=\"");

        let ct = msg.headers().content_type().unwrap().unwrap();
        k9::assert_equal!(ct.get("name").unwrap(), "日本語の添付");
    }

    #[test]
    fn attachment_filename_mess_fastmail() {
        let message = concat!(
            "Content-Disposition: attachment;\n",
            "  filename*0*=utf-8''%E6%97%A5%E6%9C%AC%E8%AA%9E%E3%81%AE%E6%B7%BB%E4%BB%98;\n",
            "  filename*1*=.txt\n",
            "Content-Type: text/plain;\n",
            "   name=\"=?UTF-8?Q?=E6=97=A5=E6=9C=AC=E8=AA=9E=E3=81=AE=E6=B7=BB=E4=BB=98.txt?=\"\n",
            "   x-name=\"=?UTF-8?Q?=E6=97=A5=E6=9C=AC=E8=AA=9E=E3=81=AE=E6=B7=BB=E4=BB=98.txt?=bork\"\n",
            "\n\n"
        );
        let msg = MimePart::parse(message).unwrap();

        let cd = msg.headers().content_disposition().unwrap().unwrap();
        k9::assert_equal!(cd.get("filename").unwrap(), "日本語の添付.txt");

        let ct = msg.headers().content_type().unwrap().unwrap();
        eprintln!("{ct:#?}");
        k9::assert_equal!(ct.get("name").unwrap(), "日本語の添付.txt");
        k9::assert_equal!(
            ct.get("x-name").unwrap(),
            "=?UTF-8?Q?=E6=97=A5=E6=9C=AC=E8=AA=9E=E3=81=AE=E6=B7=BB=E4=BB=98.txt?=bork"
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
            msg.rebuild(None).unwrap().to_message_string(),
            r#"
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
Content-Transfer-Encoding: quoted-printable\r
From: "Keith Moore" <moore@cs.utk.edu>\r
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
A Group:"Ed Jones" <c@a.test>,\r
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
            encoding: None,
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
                encoding: None,
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
            encoding: Rfc2231,
            value: "This%20is%20even%20more%20",
        },
        MimeParameter {
            name: "title",
            section: Some(
                1,
            ),
            mime_charset: None,
            mime_language: None,
            encoding: Rfc2231,
            value: "%2A%2A%2Afun%2A%2A%2A%20",
        },
        MimeParameter {
            name: "title",
            section: Some(
                2,
            ),
            mime_charset: None,
            mime_language: None,
            encoding: None,
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

    #[test]
    fn arc_authentication_results_1() {
        let ar = Header::with_name_value(
            "ARC-Authentication-Results",
            "i=3; clochette.example.org; spf=fail
    smtp.from=jqd@d1.example; dkim=fail (512-bit key)
    header.i=@d1.example; dmarc=fail; arc=pass (as.2.gmail.example=pass,
    ams.2.gmail.example=pass, as.1.lists.example.org=pass,
    ams.1.lists.example.org=fail (message has been altered))",
        );
        let ar = match ar.as_arc_authentication_results() {
            Err(err) => panic!("\n{err}"),
            Ok(ar) => ar,
        };

        k9::snapshot!(
            &ar,
            r#"
ARCAuthenticationResults {
    instance: 3,
    serv_id: "clochette.example.org",
    version: None,
    results: [
        AuthenticationResult {
            method: "spf",
            method_version: None,
            result: "fail",
            reason: None,
            props: {
                "smtp.from": "jqd@d1.example",
            },
        },
        AuthenticationResult {
            method: "dkim",
            method_version: None,
            result: "fail",
            reason: None,
            props: {
                "header.i": "@d1.example",
            },
        },
        AuthenticationResult {
            method: "dmarc",
            method_version: None,
            result: "fail",
            reason: None,
            props: {},
        },
        AuthenticationResult {
            method: "arc",
            method_version: None,
            result: "pass",
            reason: None,
            props: {},
        },
    ],
}
"#
        );
    }
}
