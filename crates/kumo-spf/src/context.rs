use crate::dns::{DnsError, Lookup};
use crate::record::Record;
use crate::spec::MacroSpec;
use crate::{SpfDisposition, SpfResult};
use std::net::IpAddr;
use std::time::SystemTime;

pub struct SpfContext<'a> {
    pub(crate) sender: &'a str,
    pub(crate) local_part: &'a str,
    pub(crate) sender_domain: &'a str,
    pub(crate) domain: &'a str,
    pub(crate) client_ip: IpAddr,
    pub(crate) now: SystemTime,
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

    pub(crate) fn domain(&self, spec: Option<&MacroSpec>) -> Result<String, SpfResult> {
        let Some(spec) = spec else {
            return Ok(self.domain.to_owned());
        };

        spec.expand(self).map_err(|err| SpfResult {
            disposition: SpfDisposition::TempError,
            context: format!("error evaluating domain spec: {err}"),
        })
    }
}

#[cfg(test)]
mod test {
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
