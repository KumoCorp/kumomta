#[derive(Debug)]
pub struct QueueNameComponents<'a> {
    pub campaign: Option<&'a str>,
    pub tenant: Option<&'a str>,
    pub domain: &'a str,
    pub routing_domain: Option<&'a str>,
}

fn crack_domain(domain: &str) -> (&str, Option<&str>) {
    match domain.split_once('!') {
        Some((domain, routing_domain)) => (domain, Some(routing_domain)),
        None => (domain, None),
    }
}

impl<'a> QueueNameComponents<'a> {
    pub fn parse(name: &'a str) -> Self {
        match name.split_once('@') {
            Some((prefix, domain)) => match prefix.split_once(':') {
                Some((campaign, tenant)) => {
                    let (domain, routing_domain) = crack_domain(domain);
                    Self {
                        campaign: Some(campaign),
                        tenant: Some(tenant),
                        domain,
                        routing_domain,
                    }
                }
                None => {
                    let (domain, routing_domain) = crack_domain(domain);
                    Self {
                        campaign: None,
                        tenant: Some(prefix),
                        domain,
                        routing_domain,
                    }
                }
            },
            None => {
                let (domain, routing_domain) = crack_domain(name);
                Self {
                    campaign: None,
                    tenant: None,
                    domain,
                    routing_domain,
                }
            }
        }
    }

    pub fn to_string(&self) -> String {
        Self::format(
            self.campaign.as_deref(),
            self.tenant.as_deref(),
            &self.domain,
            self.routing_domain.as_deref(),
        )
    }

    pub fn format<C: AsRef<str>, T: AsRef<str>, D: AsRef<str>, RD: AsRef<str>>(
        campaign: Option<C>,
        tenant: Option<T>,
        domain: D,
        routing_domain: Option<RD>,
    ) -> String {
        let campaign: Option<&str> = campaign.as_ref().map(|c| c.as_ref());
        let tenant: Option<&str> = tenant.as_ref().map(|c| c.as_ref());
        let routing_domain: Option<String> =
            routing_domain.as_ref().map(|c| format!("!{}", c.as_ref()));
        let routing_domain = routing_domain.as_deref().unwrap_or("");
        let domain: &str = domain.as_ref();
        match (campaign, tenant) {
            (Some(c), Some(t)) => format!("{c}:{t}@{domain}{routing_domain}"),
            (Some(c), None) => format!("{c}:@{domain}{routing_domain}"),
            (None, Some(t)) => format!("{t}@{domain}{routing_domain}"),
            (None, None) => format!("{domain}{routing_domain}"),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_routing_domain_syntax() {
        k9::assert_equal!(crack_domain("foo.com"), ("foo.com", None));
        k9::assert_equal!(
            crack_domain("foo.com!bar.com"),
            ("foo.com", Some("bar.com"))
        );
    }

    #[test]
    fn test_queue_name() {
        k9::snapshot!(
            QueueNameComponents::parse("foo.com"),
            r#"
QueueNameComponents {
    campaign: None,
    tenant: None,
    domain: "foo.com",
    routing_domain: None,
}
"#
        );
        k9::snapshot!(
            QueueNameComponents::parse("tenant@foo.com"),
            r#"
QueueNameComponents {
    campaign: None,
    tenant: Some(
        "tenant",
    ),
    domain: "foo.com",
    routing_domain: None,
}
"#
        );
        k9::snapshot!(
            QueueNameComponents::parse("campaign:@foo.com"),
            r#"
QueueNameComponents {
    campaign: Some(
        "campaign",
    ),
    tenant: Some(
        "",
    ),
    domain: "foo.com",
    routing_domain: None,
}
"#
        );
        k9::snapshot!(
            QueueNameComponents::parse("campaign:tenant@foo.com"),
            r#"
QueueNameComponents {
    campaign: Some(
        "campaign",
    ),
    tenant: Some(
        "tenant",
    ),
    domain: "foo.com",
    routing_domain: None,
}
"#
        );
        k9::snapshot!(
            QueueNameComponents::parse("campaign:tenant@foo.com!routing.com"),
            r#"
QueueNameComponents {
    campaign: Some(
        "campaign",
    ),
    tenant: Some(
        "tenant",
    ),
    domain: "foo.com",
    routing_domain: Some(
        "routing.com",
    ),
}
"#
        );
    }
}
