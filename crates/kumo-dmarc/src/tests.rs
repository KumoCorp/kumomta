use crate::types::results::DispositionWithContext;
use crate::{Disposition, DmarcContext};
use bstr::ByteSlice;
use dns_resolver::{Resolver, TestResolver};
use std::collections::BTreeMap;

struct TestData<'a> {
    from_domain: &'a str,
    mail_from_domain: &'a str,
    dkim_domains: &'a [Option<&'a str>],
    spf_result: Option<&'a str>,
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
        spf_result: None,
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
        spf_result: None,
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
        spf_result: None,
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
        spf_result: None,
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
        spf_result: None,
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
        spf_result: None,
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
        spf_result: None,
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
        spf_result: None,
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
        spf_result: Some("pass"),
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
        spf_result: Some("pass"),
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
        spf_result: Some("pass"),
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
        spf_result: Some("pass"),
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Reject);
}

#[tokio::test]
async fn dmarc_passes_when_dkim_aligns_even_if_spf_fails() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=s; aspf=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "helper.example.com",
        dkim_domains: &[Some("example.com")],
        spf_result: Some("fail"),
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Pass);
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
            spf_result: Some("pass"),
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

#[tokio::test]
async fn dmarc_check_includes_record_tags() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; sp=quarantine; adkim=s; aspf=r; pct=100; fo=1; rf=afrf; ri=3600"
                .to_string(),
        );

    let result = evaluate_ip(TestData {
        from_domain: "example.com",
        mail_from_domain: "example.net",
        dkim_domains: &[],
        spf_result: Some("fail"),
        resolver: &resolver,
    })
    .await;

    k9::assert_equal!(result.result, Disposition::Reject);
    k9::assert_equal!(
        result
            .props
            .get("policy.p".as_bytes())
            .unwrap()
            .to_str()
            .unwrap(),
        "reject"
    );
    k9::assert_equal!(
        result
            .props
            .get("policy.sp".as_bytes())
            .unwrap()
            .to_str()
            .unwrap(),
        "quarantine"
    );
    k9::assert_equal!(
        result
            .props
            .get("policy.adkim".as_bytes())
            .unwrap()
            .to_str()
            .unwrap(),
        "s"
    );
    k9::assert_equal!(
        result
            .props
            .get("policy.aspf".as_bytes())
            .unwrap()
            .to_str()
            .unwrap(),
        "r"
    );
    k9::assert_equal!(
        result
            .props
            .get("policy.pct".as_bytes())
            .unwrap()
            .to_str()
            .unwrap(),
        "100"
    );
    k9::assert_equal!(
        result
            .props
            .get("policy.fo".as_bytes())
            .unwrap()
            .to_str()
            .unwrap(),
        "1"
    );
    k9::assert_equal!(
        result
            .props
            .get("policy.rf".as_bytes())
            .unwrap()
            .to_str()
            .unwrap(),
        "afrf"
    );
    k9::assert_equal!(
        result
            .props
            .get("policy.ri".as_bytes())
            .unwrap()
            .to_str()
            .unwrap(),
        "3600"
    );
}

async fn evaluate_ip<'a>(
    TestData {
        from_domain,
        mail_from_domain,
        dkim_domains,
        spf_result,
        resolver,
    }: TestData<'a>,
) -> DispositionWithContext {
    let mut dkim_vec = vec![];

    for dkim_domain in dkim_domains {
        if let Some(dkim_domain) = dkim_domain {
            let mut map = BTreeMap::new();
            map.insert("header.d".into(), dkim_domain.to_string().into());
            map.insert("result".into(), "pass".into());

            dkim_vec.push(map);
        } else {
            let mut map = BTreeMap::new();
            map.insert("result".into(), "pass".into());
            dkim_vec.push(map);
        }
    }

    let spf_result = spf_result.map(|result| {
        let mut map = BTreeMap::new();
        map.insert("result".into(), result.into());
        map.insert(
            "smtp.mailfrom".into(),
            format!("sender@{mail_from_domain}").into(),
        );
        map
    });

    match DmarcContext::new(
        from_domain,
        Some(mail_from_domain),
        &dkim_vec,
        spf_result.as_ref(),
    ) {
        Ok(cx) => cx.check(resolver).await,
        Err(result) => result,
    }
}

#[tokio::test]
async fn dmarc_ignores_aligned_dkim_that_did_not_pass() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=s; aspf=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let mut dkim_vec = vec![];
    let mut dkim_map = BTreeMap::new();
    dkim_map.insert("header.d".into(), "example.com".into());
    dkim_map.insert("result".into(), "fail".into());
    dkim_vec.push(dkim_map);

    let spf_result = None;

    let result = match DmarcContext::new(
        "example.com",
        Some("example.com"),
        &dkim_vec,
        spf_result.as_ref(),
    ) {
        Ok(cx) => cx.check(&resolver).await,
        Err(result) => result,
    };

    k9::assert_equal!(result.result, Disposition::Reject);
}

#[tokio::test]
async fn dmarc_ignores_aligned_spf_that_did_not_pass() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "_dmarc.example.com",
            "v=DMARC1; p=reject; adkim=s; aspf=s; \
            rua=mailto:dmarc-feedback@example.com"
                .to_string(),
        );

    let dkim_vec = vec![];
    let mut spf_map = BTreeMap::new();
    spf_map.insert("result".into(), "fail".into());
    let spf_result = Some(spf_map);

    let result = match DmarcContext::new(
        "example.com",
        Some("example.com"),
        &dkim_vec,
        spf_result.as_ref(),
    ) {
        Ok(cx) => cx.check(&resolver).await,
        Err(result) => result,
    };

    k9::assert_equal!(result.result, Disposition::Reject);
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
