use crate::types::results::DmarcResultWithContext;
use crate::{DmarcContext, DmarcResult};
use dns_resolver::{Resolver, TestResolver};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};

#[tokio::test]
async fn dmarc_dkim_relaxed_subdomain() {
    let resolver = TestResolver::default().with_zone(EXAMPLE_COM).with_txt(
        "sample.example.com",
        "v=DMARC1; p=reject; adkim=r; \
            rua=mailto:dmarc-feedback@example.com"
            .to_string(),
    );

    let result = evaluate_ip(
        Ipv4Addr::LOCALHOST,
        "sample.example.com",
        "sample.example.com",
        Some("example.com"),
        &resolver,
    )
    .await;

    k9::assert_equal!(result.result, DmarcResult::Pass);
}

#[tokio::test]
async fn dmarc_dkim_strict_subdomain() {
    let resolver = TestResolver::default().with_zone(EXAMPLE_COM).with_txt(
        "example.com",
        "v=DMARC1; p=reject; adkim=s; \
            rua=mailto:dmarc-feedback@example.com"
            .to_string(),
    );

    let result = evaluate_ip(
        Ipv4Addr::LOCALHOST,
        "sample.example.com",
        "example.com",
        Some("example.com"),
        &resolver,
    )
    .await;

    k9::assert_equal!(result.result, DmarcResult::Fail);
}

#[tokio::test]
async fn dmarc_spf_relaxed_subdomain() {
    let resolver = TestResolver::default().with_zone(EXAMPLE_COM).with_txt(
        "example.com",
        "v=DMARC1; p=reject; aspf=r; \
            rua=mailto:dmarc-feedback@example.com"
            .to_string(),
    );

    let result = evaluate_ip(
        Ipv4Addr::LOCALHOST,
        "example.com",
        "helper.example.com",
        None,
        &resolver,
    )
    .await;

    k9::assert_equal!(result.result, DmarcResult::Pass);
}

#[tokio::test]
async fn dmarc_spf_strict_subdomain() {
    let resolver = TestResolver::default().with_zone(EXAMPLE_COM).with_txt(
        "example.com",
        "v=DMARC1; p=reject; aspf=s; \
            rua=mailto:dmarc-feedback@example.com"
            .to_string(),
    );

    let result = evaluate_ip(
        Ipv4Addr::LOCALHOST,
        "example.com",
        "helper.example.com",
        None,
        &resolver,
    )
    .await;

    k9::assert_equal!(result.result, DmarcResult::Fail);
}

async fn evaluate_ip(
    client_ip: impl Into<IpAddr>,
    from_domain: &str,
    mail_from_domain: &str,
    dkim_domain: Option<&str>,
    resolver: &dyn Resolver,
) -> DmarcResultWithContext {
    let dkim = if let Some(dkim_domain) = dkim_domain {
        let mut map = BTreeMap::new();
        map.insert("header.d".to_string(), dkim_domain.to_string());

        vec![map]
    } else {
        vec![]
    };

    match DmarcContext::new(from_domain, Some(mail_from_domain), client_ip.into(), &dkim) {
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
sample      A   192.0.2.67
mail-a      A   192.0.2.129
mail-b      A   192.0.2.130
www         CNAME example.com."#;
