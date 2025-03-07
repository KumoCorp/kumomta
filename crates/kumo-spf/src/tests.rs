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

// Ensure that a split spf record is joined and parsed correctly
// <https://datatracker.ietf.org/doc/html/rfc7208#section-3.3>
#[tokio::test]
async fn txt_record_joining() {
    let resolver = TestResolver::default().with_zone(
        r#"; https://datatracker.ietf.org/doc/html/rfc7208#section-3.3
$ORIGIN example.com.
@       600 TXT "v=spf1 " "?all"
            TXT "something else"
"#,
    );
    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 65]), &resolver).await;
    k9::snapshot!(
        result,
        r#"
SpfResult {
    disposition: Neutral,
    context: "matched '?all' directive",
}
"#
    );
}

#[tokio::test]
async fn txt_but_no_spf() {
    let resolver = TestResolver::default().with_zone(
        r#"; https://datatracker.ietf.org/doc/html/rfc7208#section-3.3
$ORIGIN example.com.
@       600 TXT "not spf"
"#,
    );
    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 65]), &resolver).await;
    k9::snapshot!(
        result,
        r#"
SpfResult {
    disposition: None,
    context: "no SPF records found for example.com",
}
"#
    );
}

// Ensure that we see the SPF record out of a collection of misc other TXT records
#[tokio::test]
async fn test_yahoo() {
    let resolver = TestResolver::default()
        .with_txt_multiple(
            "yahoo.com",
            vec![
                "facebook-domain-verification=gysqrcd69g0ej34f4jfn0huivkym1p".to_string(),
                "v=spf1 redirect=_spf.mail.yahoo.com".to_string(),
            ],
        )
        .with_txt(
            "_spf.mail.yahoo.com",
            "v=spf1 ptr:yahoo.com ptr:yahoo.net ?all".to_string(),
        );
    let ctx = SpfContext::new(
        "sender@yahoo.com",
        "yahoo.com",
        Ipv4Addr::from([192, 0, 2, 65]).into(),
    )
    .unwrap();
    k9::snapshot!(
        ctx.check(&resolver, true).await,
        r#"
SpfResult {
    disposition: Neutral,
    context: "matched '?all' directive",
}
"#
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
        Ok(cx) => cx.check(resolver, true).await,
        Err(result) => result,
    }
}

/// "If the <domain> is malformed (e.g., label longer than 63 characters,
/// zero-length label not at the end, etc.) or is not a multi-label
/// domain name, or if the DNS lookup returns "Name Error" (RCODE 3, also
/// known as "NXDOMAIN" [RFC2308]), check_host() immediately returns the
/// result "none".  DNS RCODEs are defined in [RFC1035].  Properly formed
/// domains are fully qualified domains as defined in [RFC1983].  That
/// is, in the DNS they are implicitly qualified relative to the root
/// (see Section 3.1 of [RFC1034]).  Internationalized domain names MUST
/// be encoded as A-labels, as described in Section 2.3 of [RFC5890].
///
/// <https://www.rfc-editor.org/rfc/rfc7208#section-4.3>
#[tokio::test]
async fn initial_processing() {
    let resolver = TestResolver::default();

    // Invalid domain
    let cx = SpfContext::new(
        "sender@example.com",
        "example..com",
        Ipv4Addr::LOCALHOST.into(),
    )
    .unwrap();
    let result = cx.check(&resolver, true).await;
    assert_eq!(result.disposition, SpfDisposition::None);
    assert_eq!(result.context, "invalid domain name: example..com");

    // NXDOMAIN
    let cx = SpfContext::new(
        "sender@example.com",
        "example.com",
        Ipv4Addr::LOCALHOST.into(),
    )
    .unwrap();
    let result = cx.check(&resolver, true).await;
    assert_eq!(result.disposition, SpfDisposition::None);
    assert_eq!(result.context, "no SPF records found for example.com");
}
