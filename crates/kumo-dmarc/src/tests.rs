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
        &[Some("example.com")],
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
        &[Some("example.com")],
        &resolver,
    )
    .await;

    k9::assert_equal!(result.result, DmarcResult::Fail);
}

#[tokio::test]
async fn dmarc_dkim_relaxed_illformed() {
    let resolver = TestResolver::default().with_zone(EXAMPLE_COM).with_txt(
        "example.com",
        "v=DMARC1; p=reject; adkim=r; \
            rua=mailto:dmarc-feedback@example.com"
            .to_string(),
    );

    let result = evaluate_ip(
        Ipv4Addr::LOCALHOST,
        "example.com",
        "example.com",
        &[None],
        &resolver,
    )
    .await;

    k9::assert_equal!(result.result, DmarcResult::Fail);
    k9::assert_equal!(result.context.contains("d="), true);
}

#[tokio::test]
async fn dmarc_dkim_strict_illformed() {
    let resolver = TestResolver::default().with_zone(EXAMPLE_COM).with_txt(
        "example.com",
        "v=DMARC1; p=reject; adkim=s; \
            rua=mailto:dmarc-feedback@example.com"
            .to_string(),
    );

    let result = evaluate_ip(
        Ipv4Addr::LOCALHOST,
        "example.com",
        "example.com",
        &[None],
        &resolver,
    )
    .await;

    k9::assert_equal!(result.result, DmarcResult::Fail);
    k9::assert_equal!(result.context.contains("d="), true);
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
        &[],
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
        &[],
        &resolver,
    )
    .await;

    k9::assert_equal!(result.result, DmarcResult::Fail);
}

#[tokio::test]
async fn dmarc_pct_rate() {
    let mut total_failures = 0;

    for _ in 0..10000 {
        let resolver = TestResolver::default().with_zone(EXAMPLE_COM).with_txt(
            "example.com",
            "v=DMARC1; p=reject; aspf=s; pct=50; \
                rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

        let result = evaluate_ip(
            Ipv4Addr::LOCALHOST,
            "example.com",
            "helper.example.com",
            &[],
            &resolver,
        )
        .await;

        if matches!(result.result, DmarcResult::Fail) {
            total_failures += 1;
        }
    }
    k9::assert_lesser_than!((total_failures - 5000i32).abs(), 1000);
}

async fn evaluate_ip(
    client_ip: impl Into<IpAddr>,
    from_domain: &str,
    mail_from_domain: &str,
    dkim_domains: &[Option<&str>],
    resolver: &dyn Resolver,
) -> DmarcResultWithContext {
    let mut dkim_vec = vec![];

    for dkim_domain in dkim_domains {
        if let Some(dkim_domain) = dkim_domain {
            let mut map = BTreeMap::new();
            map.insert("header.d".to_string(), dkim_domain.to_string());

            dkim_vec.push(map);
        } else {
            dkim_vec.push(BTreeMap::new());
        }
    }

    match DmarcContext::new(
        from_domain,
        Some(mail_from_domain),
        client_ip.into(),
        &dkim_vec,
    ) {
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
