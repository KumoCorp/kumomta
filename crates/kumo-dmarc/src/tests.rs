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
async fn dmarc_both_spf_and_dkim_fail_returns_fail() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; aspf=r; adkim=r; rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "otherdomain.com",
        dkim_domains: &[Some("otherdomain.com")],
        resolver: &resolver,
    })
    .await;

    k9::snapshot!(result.result, "Reject");
    k9::snapshot!(result.context, "DMARC: DKIM relaxed check failed");
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

    k9::snapshot!(result.result, "Pass");
}

#[tokio::test]
async fn dmarc_dkim_relaxed_subdomain_reverse() {
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
        dkim_domains: &[Some("sample.example.com")],
        resolver: &resolver,
    })
    .await;

    k9::snapshot!(result.result, "Pass");
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

    k9::snapshot!(result.result, "Pass");
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

    k9::snapshot!(result.result, "Reject");
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

    k9::snapshot!(result.result, "Quarantine");
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

    k9::snapshot!(result.result, "Pass");
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

    k9::snapshot!(result.result, "Reject");
}

#[tokio::test]
async fn dmarc_dkim_ignores_non_pass_results() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let mut dkim_fail = AuthenticationResult {
        method: "dkim".into(),
        method_version: None,
        result: "fail".into(),
        reason: None,
        props: BTreeMap::new(),
    };
    dkim_fail
        .props
        .insert("header.d".into(), "example.org".into());

    let mut dkim_pass = AuthenticationResult {
        method: "dkim".into(),
        method_version: None,
        result: "pass".into(),
        reason: None,
        props: BTreeMap::new(),
    };
    dkim_pass
        .props
        .insert("header.d".into(), "example.com".into());

    let spf_result = AuthenticationResult {
        method: "spf".into(),
        method_version: None,
        result: "pass".into(),
        reason: None,
        props: BTreeMap::new(),
    };

    let dkim_results = vec![dkim_fail, dkim_pass];
    let mut dmarc_context = DmarcContext::new(
        "example.com",
        Some("example.com"),
        &[],
        "",
        &dkim_results,
        &spf_result,
        None,
    );

    let result = dmarc_context.check(&resolver).await;

    k9::snapshot!(result.result, "Pass");
}

#[tokio::test]
async fn dmarc_dkim_continues_until_aligned_result() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let mut dkim_unaligned = AuthenticationResult {
        method: "dkim".into(),
        method_version: None,
        result: "pass".into(),
        reason: None,
        props: BTreeMap::new(),
    };
    dkim_unaligned
        .props
        .insert("header.d".into(), "example.org".into());

    let mut dkim_aligned = AuthenticationResult {
        method: "dkim".into(),
        method_version: None,
        result: "pass".into(),
        reason: None,
        props: BTreeMap::new(),
    };
    dkim_aligned
        .props
        .insert("header.d".into(), "example.com".into());

    let spf_result = AuthenticationResult {
        method: "spf".into(),
        method_version: None,
        result: "fail".into(),
        reason: None,
        props: BTreeMap::new(),
    };

    let dkim_results = vec![dkim_unaligned, dkim_aligned];
    let mut dmarc_context = DmarcContext::new(
        "example.com",
        Some("example.com"),
        &[],
        "",
        &dkim_results,
        &spf_result,
        None,
    );

    let result = dmarc_context.check(&resolver).await;

    k9::snapshot!(result.result, "Pass");
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

    k9::snapshot!(result.result, "Reject");
    k9::snapshot!(result.context, "DMARC: DKIM signature missing 'd=' tag");
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

    k9::snapshot!(result.result, "Reject");
    k9::snapshot!(result.context, "DMARC: DKIM signature missing 'd=' tag");
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

    k9::snapshot!(result.result, "Pass");
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

    k9::snapshot!(result.result, "Pass");
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

    k9::snapshot!(result.result, "Reject");
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

    k9::snapshot!(result.result, "Reject");
}

#[tokio::test]
async fn dmarc_spf_ignores_non_pass_result() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; aspf=s; adkim=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let mut dkim_pass = AuthenticationResult {
        method: "dkim".into(),
        method_version: None,
        result: "pass".into(),
        reason: None,
        props: BTreeMap::new(),
    };
    dkim_pass
        .props
        .insert("header.d".into(), "example.com".into());

    let mut spf_result = AuthenticationResult {
        method: "spf".into(),
        method_version: None,
        result: "fail".into(),
        reason: None,
        props: BTreeMap::new(),
    };
    spf_result
        .props
        .insert("smtp.mailfrom".into(), "helper.example.org".into());

    let dkim_results = vec![dkim_pass];
    let mut dmarc_context = DmarcContext::new(
        "example.com",
        Some("helper.example.org"),
        &[],
        "",
        &dkim_results,
        &spf_result,
        None,
    );

    let result = dmarc_context.check(&resolver).await;

    k9::snapshot!(result.result, "Pass");
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

    let mut spf_result = AuthenticationResult {
        method: "spf".into(),
        method_version: None,
        result: Default::default(),
        reason: None,
        props: BTreeMap::new(),
    };
    if dkim_domains.is_empty() {
        spf_result.result = "pass".into();
        spf_result
            .props
            .insert("smtp.mailfrom".into(), mail_from_domain.into());
    }

    for dkim_domain in dkim_domains {
        let mut authentication_result = AuthenticationResult {
            method: "dkim".into(),
            method_version: None,
            result: "pass".into(),
            reason: None,
            props: BTreeMap::new(),
        };

        if let Some(dkim_domain) = dkim_domain {
            authentication_result
                .props
                .insert("header.d".into(), dkim_domain.to_string().into());

            dkim_vec.push(authentication_result);
        } else {
            dkim_vec.push(authentication_result);
        }
    }

    // let reporting_info = crate::ReportingInfo {
    //     org_name: "org".into(),
    //     email: "test@test.org".into(),
    //     extra_contact_info: None,
    // };

    let mut dmarc_context = DmarcContext::new(
        from_domain,
        Some(mail_from_domain),
        &[],
        "",
        &dkim_vec,
        &spf_result,
        None, // Some(&reporting_info),
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
