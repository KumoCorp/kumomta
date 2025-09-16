use crate::types::results::DmarcResultWithContext;
use crate::{DmarcContext, DmarcResult};
use dns_resolver::{Resolver, TestResolver};
use std::net::{IpAddr, Ipv4Addr};

#[tokio::test]
async fn dmarc_all() {
    let resolver = TestResolver::default().with_zone(EXAMPLE_COM).with_txt(
        "example.com",
        "v=DMARC1; p=reject; aspf=r; \
            rua=mailto:dmarc-feedback@example.com"
            .to_string(),
    );

    let result = evaluate_ip(Ipv4Addr::LOCALHOST, &resolver).await;

    k9::assert_equal!(result.result, DmarcResult::Pass);
    k9::assert_equal!(result.context.starts_with("Success"), true);
}

async fn evaluate_ip(
    client_ip: impl Into<IpAddr>,
    resolver: &dyn Resolver,
) -> DmarcResultWithContext {
    match DmarcContext::new("sender@example.com", "example.com", client_ip.into()) {
        Ok(cx) => cx.check(resolver).await,
        Err(result) => result,
    }
}

const EXAMPLE_COM: &str = r#"; A domain with two mail servers, two hosts, and two servers
; at the domain name
$ORIGIN example.com.
@       600 MX  10 mail-a
            MX  20 mail-b
            A   192.0.2.10
            A   192.0.2.11
amy         A   192.0.2.65
bob         A   192.0.2.66
mail-a      A   192.0.2.129
mail-b      A   192.0.2.130
www         CNAME example.com."#;
