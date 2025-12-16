use crate::types::results::DispositionWithContext;
use crate::{Disposition, DmarcContext};
use dns_resolver::{Resolver, TestResolver};
use mailparsing::AuthenticationResult;
use std::collections::BTreeMap;

struct TestData<'a> {
    from_domain: &'a str,
    mail_from_domain: &'a str,
    dkim_domains: &'a [Option<&'a str>],
    resolver: &'a dyn Resolver,
}

#[tokio::test]
async fn dmarc_dkim_relaxed_subdomain() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.sample.example.com",
            "v=DMARC1; p=reject; adkim=r; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "sample.example.com",
        mail_from_domain: "sample.example.com",
        dkim_domains: &[Some("example.com")],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Pass);
}

#[tokio::test]
async fn dmarc_dkim_relaxed_subdomain_deep() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.a.b.c.sample.example.com",
            "v=DMARC1; p=reject; adkim=r; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "a.b.c.sample.example.com",
        mail_from_domain: "a.b.c.sample.example.com",
        dkim_domains: &[Some("example.com")],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Pass);
}

#[tokio::test]
async fn dmarc_dkim_relaxed_subdomain_fail() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.sample.example.com",
            "v=DMARC1; p=reject; adkim=r; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "sample.example.com",
        mail_from_domain: "sample.example.com",
        dkim_domains: &[Some("example.org")],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Reject);
}

#[tokio::test]
async fn dmarc_dkim_relaxed_subdomain_sp_quarantine_fail() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; sp=quarantine; adkim=r; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "sample.example.com",
        mail_from_domain: "sample.example.com",
        dkim_domains: &[Some("example.org")],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Quarantine);
}

#[tokio::test]
async fn dmarc_dkim_strict_subdomain() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "example.com",
        dkim_domains: &[Some("example.com")],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Pass);
}

#[tokio::test]
async fn dmarc_dkim_strict_subdomain_fail() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "sample.example.com",
        mail_from_domain: "example.com",
        dkim_domains: &[Some("example.com")],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Reject);
}

#[tokio::test]
async fn dmarc_dkim_relaxed_illformed() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=r; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "example.com",
        dkim_domains: &[None],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Reject);
    k9::assert_equal!(result.context.contains("d="), true);
}

#[tokio::test]
async fn dmarc_dkim_strict_illformed() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "example.com",
        dkim_domains: &[None],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Reject);
    k9::assert_equal!(result.context.contains("d="), true);
}

#[tokio::test]
async fn dmarc_spf_relaxed_subdomain() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; aspf=r; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "helper.example.com",
        dkim_domains: &[],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Pass);
}

#[tokio::test]
async fn dmarc_spf_relaxed_subdomain_deep() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; aspf=r; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "a.b.c.helper.example.com",
        dkim_domains: &[],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Pass);
}

#[tokio::test]
async fn dmarc_spf_relaxed_subdomain_fail() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; aspf=r; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "helper.example.org",
        dkim_domains: &[],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Reject);
}

#[tokio::test]
async fn dmarc_spf_strict_subdomain() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; aspf=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "helper.example.com",
        dkim_domains: &[],
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Reject);
}

#[tokio::test]
async fn dmarc_pct_rate() {
    let mut total_failures = 0;
    let iters = 10_000;
    let pct = 50;

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            format!(
                "v=DMARC1; p=reject; aspf=s; pct={pct}; \
                rua=mailto:dmarc-feedback@example.com"
            ),
        );

    for _ in 0..iters {
        let result = evaluate_ip(TestData {
            from_domain: "example.com",
            mail_from_domain: "helper.example.com",
            dkim_domains: &[],
            resolver: &resolver,
        })
        .await;

        if matches!(result.result, Disposition::Reject) {
            total_failures += 1;
        }
    }

    // Allow 15% slack either side
    let upper_bound = iters * (pct + 15) / 100;
    let lower_bound = iters * (pct - 15) / 100;

    k9::assert_lesser_than!(total_failures, upper_bound);
    k9::assert_greater_than!(total_failures, lower_bound);
}

async fn evaluate_ip<'a>(
    TestData {
        from_domain,
        mail_from_domain,
        dkim_domains,
        resolver,
    }: TestData<'a>,
) -> DispositionWithContext {
    let mut dkim_vec = vec![];

    let spf_result = AuthenticationResult {
        method: "spf".to_string(),
        method_version: None,
        result: String::new(),
        reason: None,
        props: BTreeMap::new(),
    };

    for dkim_domain in dkim_domains {
        let mut authentication_result = AuthenticationResult {
            method: "dkim".to_string(),
            method_version: None,
            result: String::new(),
            reason: None,
            props: BTreeMap::new(),
        };

        if let Some(dkim_domain) = dkim_domain {
            authentication_result
                .props
                .insert("header.d".to_string(), dkim_domain.to_string());

            dkim_vec.push(authentication_result);
        } else {
            dkim_vec.push(authentication_result);
        }
    }

    let dmarc_context = DmarcContext::new(
        from_domain,
        Some(mail_from_domain),
        &[],
        &dkim_vec,
        &spf_result,
        None,
    );

    dmarc_context.check(resolver).await
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
_dmarc      A   192.0.2.67
sample      A   192.0.2.68
mail-a      A   192.0.2.129
mail-b      A   192.0.2.130
www         CNAME example.com."#;
