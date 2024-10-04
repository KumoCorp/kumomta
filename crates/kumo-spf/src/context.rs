use crate::dns::{DnsError, IpDisplay, Lookup};
use crate::record::{DomainSpec, MacroElement, MacroName, Record};
use crate::{SpfDisposition, SpfResult};
use std::fmt::Write;
use std::net::IpAddr;
use std::time::SystemTime;

pub struct SpfContext<'a> {
    sender: &'a str,
    local_part: &'a str,
    sender_domain: &'a str,
    pub(crate) domain: &'a str,
    pub(crate) client_ip: IpAddr,
    now: SystemTime,
}

impl<'a> SpfContext<'a> {
    /// Create a new evaluation context.
    ///
    /// - `sender` is the "MAIL FROM" or "HELO" identity
    /// - `domain` is the domain that provides the sought-after authorization information;
    ///   initially, the domain portion of the "MAIL FROM" or "HELO" identity
    /// - `client_ip` is the IP address of the SMTP client that is emitting the mail
    pub fn new(sender: &'a str, domain: &'a str, client_ip: IpAddr) -> Result<Self, SpfResult> {
        let Some((local_part, sender_domain)) = sender.split_once('@') else {
            return Err(SpfResult {
                disposition: SpfDisposition::PermError,
                context:
                    "input sender parameter '{sender}' is missing @ sign to delimit local part and domain".to_owned(),
            });
        };

        Ok(Self {
            sender,
            local_part,
            sender_domain,
            domain,
            client_ip,
            now: SystemTime::now(),
        })
    }

    pub(crate) fn with_domain(&self, domain: &'a str) -> Self {
        Self { domain, ..*self }
    }

    pub async fn check(&self, resolver: &dyn Lookup) -> SpfResult {
        let initial_txt = match resolver.lookup_txt(self.domain).await {
            Ok(parts) => parts.join(""),
            Err(err) => {
                return SpfResult {
                    disposition: match err {
                        DnsError::NotFound(_) => SpfDisposition::None,
                        DnsError::LookupFailed(_) => SpfDisposition::TempError,
                    },
                    context: format!("{err}"),
                };
            }
        };

        match Record::parse(&initial_txt) {
            Ok(record) => record.evaluate(self, resolver).await,
            Err(err) => {
                return SpfResult {
                    disposition: SpfDisposition::PermError,
                    context: format!("failed to parse spf record: {err}"),
                }
            }
        }
    }

    pub(crate) fn domain(&self, spec: Option<&DomainSpec>) -> Result<String, SpfResult> {
        let Some(spec) = spec else {
            return Ok(self.domain.to_owned());
        };

        self.expand(&spec.elements).map_err(|err| SpfResult {
            disposition: SpfDisposition::TempError,
            context: format!("error evaluating domain spec: {err}"),
        })
    }

    pub fn expand(&self, elements: &[MacroElement]) -> Result<String, String> {
        let (mut result, mut buf) = (String::new(), String::new());
        for element in elements {
            let m = match element {
                MacroElement::Literal(t) => {
                    result.push_str(&t);
                    continue;
                }
                MacroElement::Macro(m) => m,
            };

            buf.clear();
            match m.name {
                MacroName::Sender => buf.push_str(self.sender),
                MacroName::LocalPart => buf.push_str(&self.local_part),
                MacroName::SenderDomain => buf.push_str(&self.sender_domain),
                MacroName::Domain => buf.push_str(&self.domain),
                MacroName::ReverseDns => buf.push_str(match self.client_ip.is_ipv4() {
                    true => "in-addr",
                    false => "ip6",
                }),
                MacroName::ClientIp => {
                    buf.write_fmt(format_args!("{}", self.client_ip)).unwrap();
                }
                MacroName::Ip => buf
                    .write_fmt(format_args!(
                        "{}",
                        IpDisplay {
                            ip: self.client_ip,
                            reverse: false
                        }
                    ))
                    .unwrap(),
                MacroName::CurrentUnixTimeStamp => buf
                    .write_fmt(format_args!(
                        "{}",
                        self.now
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    ))
                    .unwrap(),
                MacroName::RelayingHostName
                | MacroName::HeloDomain
                | MacroName::ValidatedDomainName => {
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::record::DomainSpec;

    #[test]
    fn test_eval() {
        // <https://datatracker.ietf.org/doc/html/rfc7208#section-7.4>

        let mut ctx = SpfContext::new(
            "strong-bad@email.example.com",
            "email.example.com",
            IpAddr::from([192, 0, 2, 3]),
        )
        .unwrap();

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
        ] {
            let spec = DomainSpec::parse(input).unwrap();
            let output = ctx.expand(&spec.elements).unwrap();
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
            let spec = DomainSpec::parse(input).unwrap();
            let output = ctx.expand(&spec.elements).unwrap();
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
            let spec = DomainSpec::parse(input).unwrap();
            let output = ctx.expand(&spec.elements).unwrap();
            k9::assert_equal!(&output, expect, "{input}");
        }
    }
}
