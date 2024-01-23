use crate::record::{MacroElement, MacroName};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::SystemTime;

#[derive(Default)]
pub struct EvalContext {
    vars: HashMap<MacroName, String>,
}

impl EvalContext {
    pub fn new() -> Self {
        let mut ctx = Self::default();

        ctx.set_var(
            MacroName::CurrentUnixTimeStamp,
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        );

        ctx
    }

    pub fn set_sender<V: ToString>(&mut self, sender: V) -> Result<(), String> {
        let sender = sender.to_string();
        let (local, domain) = sender.split_once('@').ok_or_else(|| {
            format!("invalid sender {sender} is missing @ sign to delimit local part and domain")
        })?;

        self.set_var(MacroName::LocalPart, local);
        self.set_var(MacroName::SenderDomain, domain);
        self.set_var(MacroName::Sender, sender);

        Ok(())
    }

    pub fn set_ip<IP: Into<IpAddr>>(&mut self, ip: IP) {
        let ip: IpAddr = ip.into();
        self.set_var(MacroName::ClientIp, ip);

        match ip {
            IpAddr::V4(v4) => {
                self.set_var(MacroName::Ip, v4);
            }
            IpAddr::V6(v6) => {
                // For IPv6 addresses, the "i" macro expands to a dot-format address;
                // it is intended for use in %{ir}.
                let mut ip = String::new();
                for segment in v6.segments() {
                    for b in format!("{segment:04x}").chars() {
                        if !ip.is_empty() {
                            ip.push('.');
                        }
                        ip.push(b);
                    }
                }
                self.set_var(MacroName::Ip, ip);
            }
        }
        self.set_var(
            MacroName::ReverseDns,
            if ip.is_ipv4() { "in-addr" } else { "ip6" },
        );
    }

    pub fn set_client_ip<IP: Into<IpAddr>>(&mut self, ip: IP) {
        let ip: IpAddr = ip.into();
        self.set_var(MacroName::ClientIp, ip);
    }

    pub fn set_var<V: ToString>(&mut self, name: MacroName, value: V) {
        self.vars.insert(name, value.to_string());
    }

    pub fn evaluate(&self, elements: &[MacroElement]) -> Result<String, String> {
        let mut result = String::new();
        for element in elements {
            match element {
                MacroElement::Literal(t) => {
                    result.push_str(&t);
                }
                MacroElement::Macro(m) => {
                    eprintln!("apply {m:?}");
                    let value = self
                        .vars
                        .get(&m.name)
                        .ok_or_else(|| format!("{:?} has no been set in EvalContext", m.name))?;
                    let delimiters = if m.delimiters.is_empty() {
                        "."
                    } else {
                        &m.delimiters
                    };
                    let mut tokens: Vec<&str> = value.split(|c| delimiters.contains(c)).collect();

                    if m.reverse {
                        tokens.reverse();
                    }

                    if let Some(n) = m.transformer_digits {
                        let n = n as usize;
                        while tokens.len() > n {
                            tokens.remove(0);
                        }
                    }

                    result.push_str(&tokens.join("."));
                }
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

        let mut ctx = EvalContext::new();
        ctx.set_sender("strong-bad@email.example.com").unwrap();
        ctx.set_var(MacroName::Domain, "email.example.com");

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
            let output = ctx.evaluate(&spec.elements).unwrap();
            k9::assert_equal!(&output, expect, "{input}");
        }

        ctx.set_ip("192.0.2.3".parse::<IpAddr>().unwrap());

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
            let output = ctx.evaluate(&spec.elements).unwrap();
            k9::assert_equal!(&output, expect, "{input}");
        }

        ctx.set_ip("2001:db8::cb01".parse::<IpAddr>().unwrap());
        for (input, expect) in &[
            (
                "%{ir}.%{v}._spf.%{d2}",
                "1.0.b.c.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0\
                 .0.0.0.0.8.b.d.0.1.0.0.2.ip6._spf.example.com",
            ),
            ("%{c}", "2001:db8::cb01"),
            ("%{C}", "2001:db8::cb01"),
        ] {
            let spec = DomainSpec::parse(input).unwrap();
            let output = ctx.evaluate(&spec.elements).unwrap();
            k9::assert_equal!(&output, expect, "{input}");
        }
    }
}
