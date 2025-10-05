use crate::SpfContext;
use dns_resolver::IpDisplay;
use std::fmt::{self, Write};
use std::time::SystemTime;

fn starts_with_number(input: &str) -> Result<(Option<u32>, &str), String> {
    let i = input
        .find(|c: char| !c.is_numeric() && c != '.')
        .unwrap_or(input.len());
    if i == 0 {
        return Ok((None, input));
    }
    let number = input[..i]
        .parse::<u32>()
        .map_err(|err| format!("error parsing number from {input}: {err}"))?;
    Ok((Some(number), &input[i..]))
}

#[derive(Debug)]
pub(crate) struct MacroSpec {
    elements: Vec<MacroElement>,
}

impl MacroSpec {
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
            (0x21..=0x24).contains(&c) || (0x26..=0x7e).contains(&c)
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

    pub(crate) fn expand(&self, cx: &SpfContext<'_>) -> Result<String, String> {
        let (mut result, mut buf) = (String::new(), String::new());
        for element in &self.elements {
            let m = match element {
                MacroElement::Literal(t) => {
                    result.push_str(t);
                    continue;
                }
                MacroElement::Macro(m) => m,
            };

            buf.clear();
            match m.name {
                MacroName::Sender => buf.push_str(cx.sender),
                MacroName::LocalPart => buf.push_str(cx.local_part),
                MacroName::SenderDomain => buf.push_str(cx.sender_domain),
                MacroName::Domain => buf.push_str(cx.domain),
                MacroName::ReverseDns => buf.push_str(if cx.client_ip.is_ipv4() {
                    "in-addr"
                } else {
                    "ip6"
                }),
                MacroName::ClientIp => {
                    buf.write_fmt(format_args!("{}", cx.client_ip)).unwrap();
                }
                MacroName::Ip => buf
                    .write_fmt(format_args!(
                        "{}",
                        IpDisplay {
                            ip: cx.client_ip,
                            reverse: false
                        }
                    ))
                    .unwrap(),
                MacroName::CurrentUnixTimeStamp => buf
                    .write_fmt(format_args!(
                        "{}",
                        cx.now
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    ))
                    .unwrap(),
                MacroName::HeloDomain => {
                    buf.push_str(cx.ehlo_domain.unwrap_or(""));
                }
                MacroName::RelayingHostName => {
                    buf.push_str(cx.relaying_host_name);
                }
                MacroName::ValidatedDomainName => {
                    return Err(format!("{:?} has not been implemented", m.name))
                }
            };

            let delimiters = if m.delimiters.is_empty() {
                "."
            } else {
                &m.delimiters
            };

            let mut tokens: Vec<&str> = buf.split(|c| delimiters.contains(c)).collect();

            if m.reverse {
                tokens.reverse();
            }

            if let Some(n) = m.transformer_digits {
                let n = n as usize;
                while tokens.len() > n {
                    tokens.remove(0);
                }
            }

            let output = tokens.join(".");

            if m.url_escape {
                // https://datatracker.ietf.org/doc/html/rfc7208#section-7.3:
                //   Uppercase macros expand exactly as their lowercase
                //   equivalents, and are then URL escaped.  URL escaping
                //   MUST be performed for characters not in the
                //   "unreserved" set.
                // https://datatracker.ietf.org/doc/html/rfc3986#section-2.3:
                //    unreserved  = ALPHA / DIGIT / "-" / "." / "_" / "~"
                for c in output.chars() {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' || c == '~' {
                        result.push(c);
                    } else {
                        let mut bytes = [0u8; 4];
                        for b in c.encode_utf8(&mut bytes).bytes() {
                            result.push_str(&format!("%{b:02x}"));
                        }
                    }
                }
            } else {
                result.push_str(&output);
            }
        }

        Ok(result)
    }
}

impl fmt::Display for MacroSpec {
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
enum MacroElement {
    Literal(String),
    Macro(MacroTerm),
}

#[derive(Debug)]
struct MacroTerm {
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
enum MacroName {
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

    fn as_char(&self) -> char {
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

#[cfg(test)]
mod test {
    use std::net::IpAddr;

    use super::*;
    use crate::spec::MacroSpec;

    #[test]
    fn test_eval() {
        // <https://datatracker.ietf.org/doc/html/rfc7208#section-7.4>

        let mut ctx = SpfContext::new(
            "strong-bad@email.example.com",
            "email.example.com",
            IpAddr::from([192, 0, 2, 3]),
        )
        .unwrap()
        .with_ehlo_domain(Some("mx1.example.com"))
        .with_relaying_host_name(Some("mx.mbp.com"));

        for (input, expect) in &[
            ("%{s}", "strong-bad@email.example.com"),
            ("%{o}", "email.example.com"),
            ("%{d}", "email.example.com"),
            ("%{d4}", "email.example.com"),
            ("%{d3}", "email.example.com"),
            ("%{d2}", "example.com"),
            ("%{d1}", "com"),
            ("%{dr}", "com.example.email"),
            ("%{d2r}", "example.email"),
            ("%{l}", "strong-bad"),
            ("%{l-}", "strong.bad"),
            ("%{lr}", "strong-bad"),
            ("%{lr-}", "bad.strong"),
            ("%{l1r-}", "strong"),
            ("%{h}", "mx1.example.com"),
            ("%{h2}", "example.com"),
            ("%{r}", "mx.mbp.com"),
            ("%{rr}", "com.mbp.mx"),
        ] {
            let spec = MacroSpec::parse(input).unwrap();
            let output = spec.expand(&ctx).unwrap();
            k9::assert_equal!(&output, expect, "{input}");
        }

        for (input, expect) in &[
            (
                "%{ir}.%{v}._spf.%{d2}",
                "3.2.0.192.in-addr._spf.example.com",
            ),
            ("%{lr-}.lp._spf.%{d2}", "bad.strong.lp._spf.example.com"),
            (
                "%{lr-}.lp.%{ir}.%{v}._spf.%{d2}",
                "bad.strong.lp.3.2.0.192.in-addr._spf.example.com",
            ),
            (
                "%{ir}.%{v}.%{l1r-}.lp._spf.%{d2}",
                "3.2.0.192.in-addr.strong.lp._spf.example.com",
            ),
            (
                "%{d2}.trusted-domains.example.net",
                "example.com.trusted-domains.example.net",
            ),
            ("%{c}", "192.0.2.3"),
        ] {
            let spec = MacroSpec::parse(input).unwrap();
            let output = spec.expand(&ctx).unwrap();
            k9::assert_equal!(&output, expect, "{input}");
        }

        ctx.client_ip = IpAddr::from([0x2001, 0xdb8, 0, 0, 0, 0, 0, 0xcb01]);
        for (input, expect) in &[
            (
                "%{ir}.%{v}._spf.%{d2}",
                "1.0.b.c.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0\
                 .0.0.0.0.8.b.d.0.1.0.0.2.ip6._spf.example.com",
            ),
            ("%{c}", "2001:db8::cb01"),
            ("%{C}", "2001%3adb8%3a%3acb01"),
        ] {
            let spec = MacroSpec::parse(input).unwrap();
            let output = spec.expand(&ctx).unwrap();
            k9::assert_equal!(&output, expect, "{input}");
        }
    }
}
