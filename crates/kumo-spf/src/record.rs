use crate::context::SpfContext;
use crate::dns::Lookup;
use crate::{SpfDisposition, SpfResult};
use hickory_resolver::Name;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

#[derive(Debug, Default)]
pub struct Record {
    directives: Vec<Directive>,
    redirect: Option<DomainSpec>,
    explanation: Option<DomainSpec>,
}

impl Record {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut tokens = s.split(' ');
        let version = tokens
            .next()
            .ok_or_else(|| format!("expected version in {s}"))?;
        if version != "v=spf1" {
            return Err(format!("expected SPF version 1 in {s}"));
        }

        let mut new = Self::default();
        while let Some(t) = tokens.next() {
            if let Ok(directive) = Directive::parse(t) {
                if new.redirect.is_some() || new.explanation.is_some() {
                    return Err("directive after modifier".to_owned());
                }

                new.directives.push(directive);
                continue;
            }

            if let Ok(modifier) = Modifier::parse(t) {
                match modifier {
                    Modifier::Redirect(domain) => match new.redirect {
                        Some(_) => return Err("duplicate redirect modifier".to_owned()),
                        None => new.redirect = Some(domain),
                    },
                    Modifier::Explanation(domain) => match new.explanation {
                        Some(_) => return Err("duplicate explanation modifier".to_owned()),
                        None => new.explanation = Some(domain),
                    },
                    _ => {} // "Unrecognized modifiers MUST be ignored"
                }
                continue;
            }

            return Err(format!("invalid token '{t}'"));
        }

        Ok(new)
    }

    pub async fn evaluate(&self, cx: &SpfContext<'_>, resolver: &dyn Lookup) -> SpfResult {
        let mut failed = None;
        for directive in &self.directives {
            match directive.evaluate(cx, resolver).await {
                Ok(Some(SpfResult {
                    disposition: SpfDisposition::Fail,
                    context,
                })) => {
                    failed = Some(context);
                    break;
                }
                Ok(Some(result)) => return result,
                Ok(None) => continue,
                Err(err) => return err,
            }
        }

        if let Some(domain) = &self.redirect {
            let domain = match cx.domain(Some(domain)) {
                Ok(domain) => domain,
                Err(err) => return err,
            };

            let nested = cx.with_domain(&domain);
            match Box::pin(nested.check(resolver)).await {
                SpfResult {
                    disposition: SpfDisposition::Fail,
                    context,
                } => failed = Some(context),
                result => return result,
            }
        }

        let failed = match failed {
            Some(failed) => failed,
            None => {
                return SpfResult {
                    disposition: SpfDisposition::Neutral,
                    context: "default result".to_owned(),
                }
            }
        };

        let domain = match &self.explanation {
            Some(domain) => match cx.domain(Some(domain)) {
                Ok(domain) => domain,
                Err(err) => return err,
            },
            None => return SpfResult::fail(failed),
        };

        // "If there are any DNS processing errors (any RCODE other than 0), or
        // if no records are returned, or if more than one record is returned,
        // or if there are syntax errors in the explanation string, then proceed
        // as if no "exp" modifier was given."
        let explanation = match resolver.lookup_txt(&domain).await {
            Ok(mut records) if records.len() == 1 => records.pop().unwrap(),
            Ok(_) | Err(_) => return SpfResult::fail(failed),
        };

        let spec = match DomainSpec::parse(&explanation) {
            Ok(spec) => spec,
            Err(_) => return SpfResult::fail(failed),
        };

        match cx.expand(&spec.elements) {
            Ok(explanation) => SpfResult::fail(explanation),
            Err(_) => SpfResult::fail(failed),
        }
    }
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

        Ok(Self {
            qualifier,
            mechanism: Mechanism::parse(s)?,
        })
    }

    pub async fn evaluate(
        &self,
        cx: &SpfContext<'_>,
        resolver: &dyn Lookup,
    ) -> Result<Option<SpfResult>, SpfResult> {
        let matched = match &self.mechanism {
            Mechanism::All => true,
            Mechanism::A { domain, cidr_len } => {
                let domain = cx.domain(domain.as_ref())?;
                let resolved = match resolver.lookup_ip(&domain).await {
                    Ok(ips) => ips,
                    Err(err) => {
                        return Err(SpfResult {
                            disposition: SpfDisposition::TempError,
                            context: format!("error looking up IP for {domain}: {err}"),
                        })
                    }
                };

                resolved
                    .iter()
                    .any(|&resolved_ip| cidr_len.matches(cx.client_ip, resolved_ip))
            }
            Mechanism::Mx { domain, cidr_len } => {
                let domain = cx.domain(domain.as_ref())?;
                let exchanges = match resolver.lookup_mx(&domain).await {
                    Ok(exchanges) => exchanges,
                    Err(err) => {
                        return Err(SpfResult {
                            disposition: SpfDisposition::TempError,
                            context: format!("error looking up IP for {domain}: {err}"),
                        })
                    }
                };

                let mut matched = false;
                for exchange in exchanges {
                    let resolved = match resolver.lookup_ip(&exchange.to_string()).await {
                        Ok(ips) => ips,
                        Err(err) => {
                            return Err(SpfResult {
                                disposition: SpfDisposition::TempError,
                                context: format!("error looking up IP for {exchange}: {err}"),
                            })
                        }
                    };

                    if resolved
                        .iter()
                        .any(|&resolved_ip| cidr_len.matches(cx.client_ip, resolved_ip))
                    {
                        matched = true;
                        break;
                    }
                }

                matched
            }
            Mechanism::Ip4 {
                ip4_network,
                cidr_len,
            } => DualCidrLength {
                v4: *cidr_len,
                ..Default::default()
            }
            .matches(cx.client_ip, IpAddr::V4(*ip4_network)),
            Mechanism::Ip6 {
                ip6_network,
                cidr_len,
            } => DualCidrLength {
                v6: *cidr_len,
                ..Default::default()
            }
            .matches(cx.client_ip, IpAddr::V6(*ip6_network)),
            Mechanism::Ptr { domain } => {
                let domain = match Name::from_str(&cx.domain(domain.as_ref())?) {
                    Ok(domain) => domain,
                    Err(err) => {
                        return Err(SpfResult {
                            disposition: SpfDisposition::PermError,
                            context: format!("error parsing domain name: {err}"),
                        })
                    }
                };

                let ptrs = match resolver.lookup_ptr(cx.client_ip).await {
                    Ok(ptrs) => ptrs,
                    Err(err) => {
                        return Err(SpfResult {
                            disposition: SpfDisposition::TempError,
                            context: format!("error looking up PTR for {}: {err}", cx.client_ip),
                        })
                    }
                };

                let mut matched = false;
                for ptr in ptrs.iter().filter(|ptr| domain.zone_of(ptr)) {
                    match resolver.lookup_ip(&ptr.to_string()).await {
                        Ok(ips) => {
                            if ips.iter().any(|&ip| ip == cx.client_ip) {
                                matched = true;
                                break;
                            }
                        }
                        Err(err) => {
                            return Err(SpfResult {
                                disposition: SpfDisposition::TempError,
                                context: format!("error looking up IP for {ptr}: {err}"),
                            })
                        }
                    }
                }

                matched
            }
            Mechanism::Include { domain } => {
                let domain = cx.domain(Some(domain))?;
                let nested = cx.with_domain(&domain);
                use SpfDisposition::*;
                match Box::pin(nested.check(resolver)).await {
                    SpfResult {
                        disposition: Pass, ..
                    } => true,
                    SpfResult {
                        disposition: Fail | SoftFail | Neutral,
                        ..
                    } => false,
                    SpfResult {
                        disposition: TempError,
                        context,
                    } => {
                        return Err(SpfResult {
                            disposition: TempError,
                            context: format!(
                                "temperror while evaluating include:{domain}: {context}"
                            ),
                        })
                    }
                    SpfResult {
                        disposition: disp @ PermError | disp @ None,
                        context,
                    } => {
                        return Err(SpfResult {
                            disposition: PermError,
                            context: format!("{disp} while evaluating include:{domain}: {context}"),
                        })
                    }
                }
            }
            Mechanism::Exists { domain } => {
                let domain = cx.domain(Some(domain))?;
                match resolver.lookup_ip(&domain).await {
                    Ok(ips) => ips.iter().any(|ip| ip.is_ipv4()),
                    Err(err) => {
                        return Err(SpfResult {
                            disposition: SpfDisposition::TempError,
                            context: format!("error looking up IP for {domain}: {err}"),
                        })
                    }
                }
            }
        };

        Ok(match matched {
            true => Some(SpfResult {
                disposition: SpfDisposition::from(self.qualifier),
                context: format!("matched '{self}' directive"),
            }),
            false => None,
        })
    }
}

impl fmt::Display for Directive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.qualifier != Qualifier::Pass {
            write!(f, "{}", self.qualifier.as_str())?;
        }
        write!(f, "{}", self.mechanism)
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

    fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "+",
            Self::Fail => "-",
            Self::SoftFail => "~",
            Self::Neutral => "?",
        }
    }
}

#[derive(Debug)]
pub struct DualCidrLength {
    pub v4: u8,
    pub v6: u8,
}

impl DualCidrLength {
    /// Whether the `observed` IP address (from the client's IP) matches the `specified` address
    /// (from/via the SPF record), given the specified CIDR mask lengths.
    fn matches(&self, observed: IpAddr, specified: IpAddr) -> bool {
        match (observed, specified, self) {
            (IpAddr::V4(observed), IpAddr::V4(specified), DualCidrLength { v4, .. }) => {
                let mask = u32::MAX << (32 - v4);
                let specified_masked = Ipv4Addr::from_bits(specified.to_bits() & mask);
                let observed_masked = Ipv4Addr::from(observed.to_bits() & mask);
                specified_masked == observed_masked
            }
            (IpAddr::V6(observed), IpAddr::V6(specified), DualCidrLength { v6, .. }) => {
                let mask = u128::MAX << (32 - v6);
                let specified_masked = Ipv6Addr::from_bits(specified.to_bits() & mask);
                let observed_masked = Ipv6Addr::from(observed.to_bits() & mask);
                specified_masked == observed_masked
            }
            _ => false,
        }
    }
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

impl fmt::Display for DualCidrLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.v4 == 32 && self.v6 == 128 {
            return Ok(());
        }

        write!(f, "/{}", self.v4)?;
        if self.v6 != 128 {
            write!(f, "/{}", self.v6)?;
        }

        Ok(())
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

impl fmt::Display for Mechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => write!(f, "all"),
            Self::Include { domain } => write!(f, "include:{}", domain),
            Self::A { domain, cidr_len } => {
                write!(f, "a")?;
                if let Some(domain) = domain {
                    write!(f, ":{}", domain)?;
                }
                write!(f, "{}", cidr_len)
            }
            Self::Mx { domain, cidr_len } => {
                write!(f, "mx")?;
                if let Some(domain) = domain {
                    write!(f, ":{}", domain)?;
                }
                write!(f, "{}", cidr_len)
            }
            Self::Ptr { domain } => {
                write!(f, "ptr")?;
                if let Some(domain) = domain {
                    write!(f, ":{}", domain)?;
                }
                Ok(())
            }
            Self::Ip4 {
                ip4_network,
                cidr_len,
            } => write!(f, "ip4:{}/{}", ip4_network, cidr_len),
            Self::Ip6 {
                ip6_network,
                cidr_len,
            } => write!(f, "ip6:{}/{}", ip6_network, cidr_len),
            Self::Exists { domain } => write!(f, "exists:{}", domain),
        }
    }
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
            r#"invalid token '-exists:%(ir).sbl.example.org'"#
        );
        k9::snapshot!(
            Record::parse("v=spf1 -exists:%{ir.sbl.example.org").unwrap_err(),
            r#"invalid token '-exists:%{ir.sbl.example.org'"#
        );
        k9::snapshot!(
            Record::parse("v=spf1 -exists:%{ir").unwrap_err(),
            r#"invalid token '-exists:%{ir'"#
        );

        k9::snapshot!(
            parse("v=spf1 mx -all exp=explain._spf.%{d}"),
            r#"
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: Some(
        DomainSpec {
            elements: [
                Literal(
                    "explain._spf.",
                ),
                Macro(
                    MacroTerm {
                        name: Domain,
                        transformer_digits: None,
                        url_escape: false,
                        reverse: false,
                        delimiters: "",
                    },
                ),
            ],
        },
    ),
}
"#
        );

        k9::snapshot!(
            parse("v=spf1 -exists:%{ir}.sbl.example.org"),
            r#"
Record {
    directives: [
        Directive {
            qualifier: Fail,
            mechanism: Exists {
                domain: DomainSpec {
                    elements: [
                        Macro(
                            MacroTerm {
                                name: Ip,
                                transformer_digits: None,
                                url_escape: false,
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
    ],
    redirect: None,
    explanation: None,
}
"#
        );

        k9::snapshot!(
            parse("v=spf1 +all"),
            "
Record {
    directives: [
        Directive {
            qualifier: Pass,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"
        );
        k9::snapshot!(
            parse("v=spf1 a -all"),
            "
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"
        );
        k9::snapshot!(
            parse("v=spf1 a:example.org -all"),
            r#"
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 mx -all"),
            "
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"
        );
        k9::snapshot!(
            parse("v=spf1 mx:example.org -all"),
            r#"
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 mx mx:example.org -all"),
            r#"
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 mx/30 -all"),
            "
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"
        );
        k9::snapshot!(
            parse("v=spf1 mx/30 mx:example.org/30 -all"),
            r#"
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 ptr -all"),
            "
Record {
    directives: [
        Directive {
            qualifier: Pass,
            mechanism: Ptr {
                domain: None,
            },
        },
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"
        );
        k9::snapshot!(
            parse("v=spf1 ip4:192.0.2.128/28 -all"),
            "
Record {
    directives: [
        Directive {
            qualifier: Pass,
            mechanism: Ip4 {
                ip4_network: 192.0.2.128,
                cidr_len: 28,
            },
        },
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"
        );
        k9::snapshot!(
            parse("v=spf1 include:example.com include:example.net -all"),
            r#"
Record {
    directives: [
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
        Directive {
            qualifier: Fail,
            mechanism: All,
        },
    ],
    redirect: None,
    explanation: None,
}
"#
        );
        k9::snapshot!(
            parse("v=spf1 redirect=example.org"),
            r#"
Record {
    directives: [],
    redirect: Some(
        DomainSpec {
            elements: [
                Literal(
                    "example.org",
                ),
            ],
        },
    ),
    explanation: None,
}
"#
        );
    }
}
