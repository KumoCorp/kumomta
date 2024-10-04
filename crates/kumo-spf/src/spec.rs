use std::fmt;

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
                if s.len() < 4 {
                    return Err(format!("unexpected end of input in {s}"));
                }

                let (name, url_escape) = MacroName::parse(
                    s.chars()
                        .nth(2)
                        .ok_or_else(|| format!("unexpected end of input in {s}"))?,
                )?;
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
                    url_escape,
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

impl fmt::Display for DomainSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for element in &self.elements {
            if first {
                first = false;
            } else {
                f.write_str(" ")?;
            }

            match element {
                MacroElement::Literal(lit) => write!(f, "{lit}")?,
                MacroElement::Macro(term) => write!(f, "{term}")?,
            }
        }
        Ok(())
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
    /// The output needs to be URL-escaped
    pub url_escape: bool,
    /// the `r` transformer was present
    pub reverse: bool,
    /// The list of delimiters, if any, otherwise an empty string
    pub delimiters: String,
}

impl fmt::Display for MacroTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}{}", self.name.as_char(), self.delimiters)?;
        if let Some(digits) = self.transformer_digits {
            write!(f, "{}", digits)?;
        }
        if self.reverse {
            f.write_str("r")?;
        }
        if self.url_escape {
            f.write_str("/")?;
        }
        Ok(())
    }
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
    fn parse(c: char) -> Result<(Self, bool), String> {
        let escape = c.is_ascii_uppercase();
        Ok((
            match c.to_ascii_lowercase() {
                's' => Self::Sender,
                'l' => Self::LocalPart,
                'o' => Self::SenderDomain,
                'd' => Self::Domain,
                'i' => Self::Ip,
                'p' => Self::ValidatedDomainName,
                'v' => Self::ReverseDns,
                'h' => Self::HeloDomain,
                'c' => Self::ClientIp,
                'r' => Self::RelayingHostName,
                't' => Self::CurrentUnixTimeStamp,
                _ => return Err(format!("invalid macro name {c}")),
            },
            escape,
        ))
    }

    pub fn as_char(&self) -> char {
        match self {
            Self::Sender => 's',
            Self::LocalPart => 'l',
            Self::SenderDomain => 'o',
            Self::Domain => 'd',
            Self::Ip => 'i',
            Self::ValidatedDomainName => 'p',
            Self::ReverseDns => 'v',
            Self::HeloDomain => 'h',
            Self::ClientIp => 'c',
            Self::RelayingHostName => 'r',
            Self::CurrentUnixTimeStamp => 't',
        }
    }
}
