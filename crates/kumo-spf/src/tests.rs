use crate::{SpfContext, SpfDisposition, SpfResult};
use dns_resolver::{Resolver, TestResolver};
use std::net::{IpAddr, Ipv4Addr};

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
#[tokio::test]
async fn all() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_txt("example.com", "v=spf1 +all".to_string());

    let result = evaluate_ip(Ipv4Addr::LOCALHOST, &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'all' directive".to_owned(),
        },
        "{result:?}"
    );
}

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
#[tokio::test]
async fn ip() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_txt("example.com", "v=spf1 a -all".to_string());

    let result = evaluate_ip(Ipv4Addr::LOCALHOST, &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 10]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'a' directive".to_owned(),
        },
        "{result:?}"
    );

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_zone(EXAMPLE_ORG)
        .with_txt("example.com", "v=spf1 a:example.org -all".to_string());

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 10]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );
}

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
#[tokio::test]
async fn mx() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_txt("example.com", "v=spf1 mx -all".to_string());

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 129]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'mx' directive".to_owned(),
        },
        "{result:?}"
    );

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_zone(EXAMPLE_ORG)
        .with_txt("example.com", "v=spf1 mx:example.org -all".to_string());

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 140]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'mx:example.org' directive".to_owned(),
        },
        "{result:?}"
    );

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_zone(EXAMPLE_ORG)
        .with_txt(
            "example.com",
            "v=spf1 mx/30 mx:example.org/30 -all".to_string(),
        );

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 131]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'mx/30' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 141]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'mx:example.org/30' directive".to_owned(),
        },
        "{result:?}"
    );
}

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
#[tokio::test]
async fn ip4() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_txt("example.com", "v=spf1 ip4:192.0.2.128/28 -all".to_string());

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 65]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 129]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'ip4:192.0.2.128/28' directive".to_owned(),
        },
        "{result:?}"
    );
}

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
#[tokio::test]
async fn ptr() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_zone(ADDR_192)
        .with_zone(ADDR_10)
        .with_txt("example.com", "v=spf1 ptr -all".to_string());

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 65]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'ptr' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 140]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = evaluate_ip(Ipv4Addr::from([10, 0, 0, 4]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );
}

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A
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

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A
const EXAMPLE_ORG: &str = r#"; A related domain
$ORIGIN example.org.
@       600 MX  10 mail-c
mail-c      A   192.0.2.140"#;

const ADDR_192: &str = r#"; The reverse IP for those addresses
$ORIGIN 2.0.192.in-addr.arpa.
10      600 PTR example.com.
11          PTR example.com.
65          PTR amy.example.com.
66          PTR bob.example.com.
129         PTR mail-a.example.com.
130         PTR mail-b.example.com.
140         PTR mail-c.example.org."#;

const ADDR_10: &str = r#"; A rogue reverse IP domain that claims to be
; something it's not
$ORIGIN 0.0.10.in-addr.arpa.
4       600 PTR bob.example.com."#;

async fn evaluate_ip(client_ip: impl Into<IpAddr>, resolver: &dyn Resolver) -> SpfResult {
    match SpfContext::new("sender@example.com", "example.com", client_ip.into()) {
        Ok(cx) => cx.check(resolver).await,
        Err(result) => result,
    }
}
