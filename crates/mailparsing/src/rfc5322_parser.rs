use crate::headermap::EncodeHeaderValue;
use crate::{MailParsingError, Result, SharedString};
use charset::Charset;
use pest::iterators::{Pair, Pairs};
use pest::Parser as _;
use pest_derive::*;

#[derive(Parser)]
#[grammar = "rfc5322.pest"]
pub struct Parser;

impl Parser {
    pub fn parse_mailbox_list_header(text: &str) -> Result<MailboxList> {
        let mut pairs = Self::parse(Rule::parse_mailbox_list, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        let mut result: Vec<Mailbox> = vec![];

        while let Some(pair) = pairs.next() {
            result.push(Self::parse_mailbox(pair.into_inner())?);
        }

        Ok(MailboxList(result))
    }

    fn parse_mailbox_list(pairs: Pairs<Rule>) -> Result<MailboxList> {
        let mut result: Vec<Mailbox> = vec![];

        for p in pairs {
            result.push(Self::parse_mailbox(p.into_inner())?);
        }

        Ok(MailboxList(result))
    }

    pub fn parse_mailbox_header(text: &str) -> Result<Mailbox> {
        let pairs = Self::parse(Rule::parse_mailbox, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        Self::parse_mailbox(pairs)
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

    pub fn parse_unstructured_header(text: &str) -> Result<String> {
        let mut pairs = Self::parse(Rule::parse_unstructured, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        eprintln!("parse_unstructured_header: {text:?}");
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
    pub address: String,
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
}
