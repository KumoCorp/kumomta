use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Debug)]
pub struct Record {
    terms: Vec<Term>,
}

impl Record {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut terms = vec![];

        let mut tokens = s.split(' ');

        let version = tokens
            .next()
            .ok_or_else(|| format!("expected version in {s}"))?;
        if version != "v=spf1" {
            return Err(format!("expected SPF version 1 in {s}"));
        }

        while let Some(t) = tokens.next() {
            match (Directive::parse(t), Modifier::parse(t)) {
                (Ok(q), _) => terms.push(Term::Directive(q)),
                (_, Ok(m)) => terms.push(Term::Modifier(m)),
                wtf => return Err(format!("unexpected result: {wtf:?} while parsing {t}")),
            }
        }

        Ok(Self { terms })
    }
}

#[derive(Debug)]
pub enum Term {
    Directive(Directive),
    Modifier(Modifier),
}

#[derive(Debug)]
pub struct Directive {
    pub qualifier: Qualifier,
    pub mechanism: Mechanism,
}

impl Directive {
    fn parse(s: &str) -> Result<Self, String> {
        let mut qualifier = Qualifier::default();
        let s = match Qualifier::parse(&s[0..1]) {
            Some(q) => {
                qualifier = q;
                &s[1..]
            }
            None => s,
        };

        let mechanism = Mechanism::parse(s)?;

        Ok(Self {
            qualifier,
            mechanism,
        })
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qualifier {
    /// `+`
    #[default]
    Pass,
    /// `-`
    Fail,
    /// `~`
    SoftFail,
    /// `?`
    Neutral,
}

impl Qualifier {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "+" => Self::Pass,
            "-" => Self::Fail,
            "~" => Self::SoftFail,
            "?" => Self::Neutral,
            _ => return None,
        })
    }
}

#[derive(Debug)]
pub struct DualCidrLength {
    pub v4: u8,
    pub v6: u8,
}

impl Default for DualCidrLength {
    fn default() -> Self {
        Self { v4: 32, v6: 128 }
    }
}

impl DualCidrLength {
    fn parse_from_end(s: &str) -> Result<(&str, Self), String> {
        match s.rsplit_once('/') {
            Some((left, right)) => {
                let right_cidr: u8 = right
                    .parse()
                    .map_err(|err| format!("invalid dual-cidr-length in {s}: {err}"))?;

                if left.ends_with('/') {
                    // we have another cidr length
                    if let Some((prefix, v4cidr)) = left[0..left.len() - 1].rsplit_once('/') {
                        let left_cidr: u8 = v4cidr.parse().map_err(|err| {
                            format!(
                                "invalid dual-cidr-length in {s}: parsing v4 cidr portion: {err}"
                            )
                        })?;
                        return Ok((
                            prefix,
                            Self {
                                v4: left_cidr,
                                v6: right_cidr,
                            },
                        ));
                    }
                }
                Ok((
                    left,
                    Self {
                        v4: right_cidr,
                        ..Self::default()
                    },
                ))
            }
            None => Ok((s, Self::default())),
        }
    }
}

#[derive(Debug)]
pub enum Mechanism {
    All,
    Include {
        domain: DomainSpec,
    },
    A {
        domain: Option<DomainSpec>,
        cidr_len: DualCidrLength,
    },
    Mx {
        domain: Option<DomainSpec>,
        cidr_len: DualCidrLength,
    },
    Ptr {
        domain: Option<DomainSpec>,
    },
    Ip4 {
        ip4_network: Ipv4Addr,
        cidr_len: u8,
    },
    Ip6 {
        ip6_network: Ipv6Addr,
        cidr_len: u8,
    },
    Exists {
        domain: DomainSpec,
    },
}

fn starts_with_number(input: &str) -> Result<(Option<u32>, &str), String> {
    let i = input
        .find(|c: char| !c.is_numeric() && c != '.')
        .unwrap_or_else(|| input.len());
    if i == 0 {
        return Ok((None, input));
    }
    let number = input[..i]
        .parse::<u32>()
        .map_err(|err| format!("error parsing number from {input}: {err}"))?;
    Ok((Some(number), &input[i..]))
}

fn starts_with_ident<'a>(s: &'a str, ident: &str) -> Option<&'a str> {
    if s.len() < ident.len() {
        return None;
    }

    if s[0..ident.len()].eq_ignore_ascii_case(ident) {
        Some(&s[ident.len()..])
    } else {
        None
    }
}

impl Mechanism {
    fn parse(s: &str) -> Result<Self, String> {
        if s.eq_ignore_ascii_case("all") {
            return Ok(Self::All);
        }

        if let Some(spec) = starts_with_ident(s, "include:") {
            return Ok(Self::Include {
                domain: DomainSpec::parse(spec)?,
            });
        }

        if let Some(remain) = starts_with_ident(s, "a") {
            let (remain, cidr_len) = DualCidrLength::parse_from_end(remain)?;

            let domain = if let Some(spec) = remain.strip_prefix(":") {
                Some(DomainSpec::parse(spec)?)
            } else if remain.is_empty() {
                None
            } else {
                return Err(format!("invalid 'a' mechanism: {s}"));
            };

            return Ok(Self::A { domain, cidr_len });
        }
        if let Some(remain) = starts_with_ident(s, "mx") {
            let (remain, cidr_len) = DualCidrLength::parse_from_end(remain)?;

            let domain = if let Some(spec) = remain.strip_prefix(":") {
                Some(DomainSpec::parse(spec)?)
            } else if remain.is_empty() {
                None
            } else {
                return Err(format!("invalid 'mx' mechanism: {s}"));
            };

            return Ok(Self::Mx { domain, cidr_len });
        }
        if let Some(remain) = starts_with_ident(s, "ptr") {
            let domain = if let Some(spec) = remain.strip_prefix(":") {
                Some(DomainSpec::parse(spec)?)
            } else if remain.is_empty() {
                None
            } else {
                return Err(format!("invalid 'ptr' mechanism: {s}"));
            };

            return Ok(Self::Ptr { domain });
        }
        if let Some(remain) = starts_with_ident(s, "ip4:") {
            let (addr, len) = remain
                .split_once('/')
                .ok_or_else(|| format!("invalid 'ip4' mechanism: {s}"))?;
            let ip4_network = addr
                .parse()
                .map_err(|err| format!("invalid 'ip4' mechanism: {s}: {err}"))?;
            let cidr_len = len
                .parse()
                .map_err(|err| format!("invalid 'ip4' mechanism: {s}: {err}"))?;

            return Ok(Self::Ip4 {
                ip4_network,
                cidr_len,
            });
        }
        if let Some(remain) = starts_with_ident(s, "ip6:") {
            let (addr, len) = remain
                .split_once('/')
                .ok_or_else(|| format!("invalid 'ip6' mechanism: {s}"))?;
            let ip6_network = addr
                .parse()
                .map_err(|err| format!("invalid 'ip6' mechanism: {s}: {err}"))?;
            let cidr_len = len
                .parse()
                .map_err(|err| format!("invalid 'ip6' mechanism: {s}: {err}"))?;

            return Ok(Self::Ip6 {
                ip6_network,
                cidr_len,
            });
        }
        if let Some(spec) = starts_with_ident(s, "exists:") {
            return Ok(Self::Exists {
                domain: DomainSpec::parse(spec)?,
            });
        }

        Err(format!("invalid mechanism {s}"))
    }
}

#[derive(Debug)]
pub enum Modifier {
    Redirect(DomainSpec),
    Explanation(DomainSpec),
    Unknown {
        name: String,
        macro_string: DomainSpec,
    },
}

impl Modifier {
    fn parse(s: &str) -> Result<Self, String> {
        if let Some(spec) = starts_with_ident(s, "redirect=") {
            return Ok(Self::Redirect(DomainSpec::parse(spec)?));
        }
        if let Some(spec) = starts_with_ident(s, "exp=") {
            return Ok(Self::Explanation(DomainSpec::parse(spec)?));
        }

        let (name, value) = s
            .split_once('=')
            .ok_or_else(|| format!("invalid modifier {s}"))?;

        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            && name.chars().next().unwrap().is_ascii_alphabetic();
        if !valid {
            return Err(format!("modifier name '{name}' is invalid"));
        }

        Ok(Self::Unknown {
            name: name.to_string(),
            macro_string: DomainSpec::parse(value)?,
        })
    }
}

#[derive(Debug)]
pub struct DomainSpec {
    pub(crate) elements: Vec<MacroElement>,
}

impl DomainSpec {
    pub(crate) fn parse(s: &str) -> Result<Self, String> {
        let mut elements = vec![];

        fn add_literal(elements: &mut Vec<MacroElement>, literal: &str) {
            match elements.last_mut() {
                Some(MacroElement::Literal(prior)) => {
                    prior.push_str(literal);
                }
                _ => {
                    elements.push(MacroElement::Literal(literal.to_string()));
                }
            }
        }

        fn is_macro_literal(c: char) -> bool {
            let c = c as u32;
            (c >= 0x21 && c <= 0x24) || (c >= 0x26 && c <= 0x7e)
        }

        let mut s = s;
        while !s.is_empty() {
            if s.starts_with("%%") {
                add_literal(&mut elements, "%");
                s = &s[2..];
                continue;
            }
            if s.starts_with("%_") {
                add_literal(&mut elements, " ");
                s = &s[2..];
                continue;
            }
            if s.starts_with("%-") {
                add_literal(&mut elements, "%20");
                s = &s[2..];
                continue;
            }
            if s.starts_with("%{") {
                let name = MacroName::parse(&s[2..3])?;
                let mut transformer_digits = None;
                let mut reverse = false;

                let remain = if let Ok((n, r)) = starts_with_number(&s[3..]) {
                    transformer_digits = n;
                    r
                } else {
                    &s[3..]
                };

                let delimiters = if remain.starts_with('r') {
                    reverse = true;
                    &remain[1..]
                } else {
                    remain
                };

                let (delimiters, remain) = delimiters
                    .split_once('}')
                    .ok_or_else(|| format!("expected '}}' to close macro in {s}"))?;

                elements.push(MacroElement::Macro(MacroTerm {
                    name,
                    transformer_digits,
                    reverse,
                    delimiters: delimiters.to_string(),
                }));

                s = remain;
                continue;
            }

            if !is_macro_literal(s.chars().next().unwrap()) {
                return Err(format!("invalid macro char in {s}"));
            }

            add_literal(&mut elements, &s[0..1]);
            s = &s[1..];
        }

        Ok(Self { elements })
    }
}

#[derive(Debug)]
pub enum MacroElement {
    Literal(String),
    Macro(MacroTerm),
}

#[derive(Debug)]
pub struct MacroTerm {
    pub name: MacroName,
    /// digits were present in the transformer section
    pub transformer_digits: Option<u32>,
    /// the `r` transformer was present
    pub reverse: bool,
    /// The list of delimiters, if any, otherwise an empty string
    pub delimiters: String,
}

#[derive(Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub enum MacroName {
    /// `s` - <sender>
    Sender,
    /// `l` - local-part of <sender>
    LocalPart,
    /// `o` - domain of <sender>
    SenderDomain,
    /// `d` - <domain>
    Domain,
    /// `i` - <ip>
    Ip,
    /// `p` - the validated domain name of <ip> (do not use)
    ValidatedDomainName,
    /// `v` the string `in-addr` if <ip> is ipv4, or `ip6` is <ip> is ipv6
    ReverseDns,
    /// `h` the HELO/EHLO domain
    HeloDomain,
    /// `c` - only in "exp" text: the SMTP client IP (easily readable format)
    ClientIp,
    /// `r` - only in "exp" text: domain name of host performing the check
    RelayingHostName,
    /// `t` - only in "exp" text: the current timestamp
    CurrentUnixTimeStamp,
}

impl MacroName {
    fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "s" => Self::Sender,
            "l" => Self::LocalPart,
            "o" => Self::SenderDomain,
            "d" => Self::Domain,
            "i" => Self::Ip,
            "p" => Self::ValidatedDomainName,
            "v" => Self::ReverseDns,
            "h" => Self::HeloDomain,
            "c" => Self::ClientIp,
            "r" => Self::RelayingHostName,
            "t" => Self::CurrentUnixTimeStamp,
            _ => return Err(format!("invalid macro name {s}")),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn parse(s: &str) -> Record {
        eprintln!("**\n{s}");
        match Record::parse(s) {
            Ok(r) => r,
            Err(err) => panic!("{err}: {s}"),
        }
    }

    #[test]
    fn test_parse() {
        k9::snapshot!(
            Record::parse("v=spf1 -exists:%(ir).sbl.example.org").unwrap_err(),
            r#"unexpected result: (Err("invalid macro char in %(ir).sbl.example.org"), Err("invalid modifier -exists:%(ir).sbl.example.org")) while parsing -exists:%(ir).sbl.example.org"#
        );
        k9::snapshot!(
            Record::parse("v=spf1 -exists:%{ir.sbl.example.org").unwrap_err(),
            r#"unexpected result: (Err("expected '}' to close macro in %{ir.sbl.example.org"), Err("invalid modifier -exists:%{ir.sbl.example.org")) while parsing -exists:%{ir.sbl.example.org"#
        );
        k9::snapshot!(
            Record::parse("v=spf1 -exists:%{ir").unwrap_err(),
            r#"unexpected result: (Err("expected '}' to close macro in %{ir"), Err("invalid modifier -exists:%{ir")) while parsing -exists:%{ir"#
        );

        k9::snapshot!(
            parse("v=spf1 mx -all exp=explain._spf.%{d}"),
            r#"
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Mx {
                    domain: None,
                    cidr_len: DualCidrLength {
                        v4: 32,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
        Modifier(
            Explanation(
                DomainSpec {
                    elements: [
                        Literal(
                            "explain._spf.",
                        ),
                        Macro(
                            MacroTerm {
                                name: Domain,
                                transformer_digits: None,
                                reverse: false,
                                delimiters: "",
                            },
                        ),
                    ],
                },
            ),
        ),
    ],
}
"#
        );

        k9::snapshot!(
            parse("v=spf1 -exists:%{ir}.sbl.example.org"),
            r#"
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: Exists {
                    domain: DomainSpec {
                        elements: [
                            Macro(
                                MacroTerm {
                                    name: Ip,
                                    transformer_digits: None,
                                    reverse: true,
                                    delimiters: "",
                                },
                            ),
                            Literal(
                                ".sbl.example.org",
                            ),
                        ],
                    },
                },
            },
        ),
    ],
}
"#
        );

        k9::snapshot!(
            parse("v=spf1 +all"),
            "
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: All,
            },
        ),
    ],
}
"
        );
        k9::snapshot!(
            parse("v=spf1 a -all"),
            "
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: A {
                    domain: None,
                    cidr_len: DualCidrLength {
                        v4: 32,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"
        );
        k9::snapshot!(
            parse("v=spf1 a:example.org -all"),
            r#"
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: A {
                    domain: Some(
                        DomainSpec {
                            elements: [
                                Literal(
                                    "example.org",
                                ),
                            ],
                        },
                    ),
                    cidr_len: DualCidrLength {
                        v4: 32,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 mx -all"),
            "
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Mx {
                    domain: None,
                    cidr_len: DualCidrLength {
                        v4: 32,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"
        );
        k9::snapshot!(
            parse("v=spf1 mx:example.org -all"),
            r#"
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Mx {
                    domain: Some(
                        DomainSpec {
                            elements: [
                                Literal(
                                    "example.org",
                                ),
                            ],
                        },
                    ),
                    cidr_len: DualCidrLength {
                        v4: 32,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 mx mx:example.org -all"),
            r#"
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Mx {
                    domain: None,
                    cidr_len: DualCidrLength {
                        v4: 32,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Mx {
                    domain: Some(
                        DomainSpec {
                            elements: [
                                Literal(
                                    "example.org",
                                ),
                            ],
                        },
                    ),
                    cidr_len: DualCidrLength {
                        v4: 32,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 mx/30 -all"),
            "
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Mx {
                    domain: None,
                    cidr_len: DualCidrLength {
                        v4: 30,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"
        );
        k9::snapshot!(
            parse("v=spf1 mx/30 mx:example.org/30 -all"),
            r#"
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Mx {
                    domain: None,
                    cidr_len: DualCidrLength {
                        v4: 30,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Mx {
                    domain: Some(
                        DomainSpec {
                            elements: [
                                Literal(
                                    "example.org",
                                ),
                            ],
                        },
                    ),
                    cidr_len: DualCidrLength {
                        v4: 30,
                        v6: 128,
                    },
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 ptr -all"),
            "
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Ptr {
                    domain: None,
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"
        );
        k9::snapshot!(
            parse("v=spf1 ip4:192.0.2.128/28 -all"),
            "
Record {
    terms: [
        Directive(
            Directive {
                qualifier: Pass,
                mechanism: Ip4 {
                    ip4_network: 192.0.2.128,
                    cidr_len: 28,
                },
            },
        ),
        Directive(
            Directive {
                qualifier: Fail,
                mechanism: All,
            },
        ),
    ],
}
"
        );
        k9::snapshot!(
            Record::parse("v=spf1 include:example.com include:example.net -all"),
            r#"
Ok(
    Record {
        terms: [
            Directive(
                Directive {
                    qualifier: Pass,
                    mechanism: Include {
                        domain: DomainSpec {
                            elements: [
                                Literal(
                                    "example.com",
                                ),
                            ],
                        },
                    },
                },
            ),
            Directive(
                Directive {
                    qualifier: Pass,
                    mechanism: Include {
                        domain: DomainSpec {
                            elements: [
                                Literal(
                                    "example.net",
                                ),
                            ],
                        },
                    },
                },
            ),
            Directive(
                Directive {
                    qualifier: Fail,
                    mechanism: All,
                },
            ),
        ],
    },
)
"#
        );
        k9::snapshot!(
            Record::parse("v=spf1 redirect=example.org"),
            r#"
Ok(
    Record {
        terms: [
            Modifier(
                Redirect(
                    DomainSpec {
                        elements: [
                            Literal(
                                "example.org",
                            ),
                        ],
                    },
                ),
            ),
        ],
    },
)
"#
        );
    }
}
