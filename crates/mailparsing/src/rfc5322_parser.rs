use crate::{MailParsingError, Result};
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

    pub fn parse_mailbox_header(text: &str) -> Result<Mailbox> {
        let pairs = Self::parse(Rule::parse_mailbox, text)
            .map_err(|err| MailParsingError::HeaderParse(format!("{err:#}")))?
            .next()
            .unwrap()
            .into_inner();

        Self::parse_mailbox(pairs)
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
                        "Unhandled {rule:?} {p:#?} in parse_domain"
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
                        "Unhandled {rule:?} {p:#?} in parse_domain_literal"
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
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unhandled {rule:?} {p:#?} in parse_angle_addr"
                    )))
                }
            }
        }
        Err(MailParsingError::HeaderParse(
            "unreachable end of loop in parse_angle_addr".to_string(),
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

    fn parse_encoded_word(pair: Pair<Rule>) -> Result<String> {
        let mut charset = String::new();
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
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unhandled {rule:?} {p:#?} in parse_atom"
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
                rule => {
                    return Err(MailParsingError::HeaderParse(format!(
                        "Unhandled {rule:?} in parse_phrase"
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
pub struct MailboxList(Vec<Mailbox>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    pub name: Option<String>,
    pub address: String,
}

#[cfg(test)]
mod test {
    use crate::MimePart;

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
    fn rfc2047() {
        let message = "From: =?US-ASCII?Q?Keith_Moore?= <moore@cs.utk.edu>\n\n\n";
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
    }
}
