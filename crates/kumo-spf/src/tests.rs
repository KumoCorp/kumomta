use crate::{SpfContext, SpfDisposition, SpfResult};
use dns_resolver::{Resolver, TestResolver};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
#[tokio::test]
async fn all() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
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
async fn a() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
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
        .unwrap()
        .with_zone(EXAMPLE_ORG)
        .unwrap()
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
        .unwrap()
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
        .unwrap()
        .with_zone(EXAMPLE_ORG)
        .unwrap()
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
        .unwrap()
        .with_zone(EXAMPLE_ORG)
        .unwrap()
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

#[tokio::test]
async fn underscores() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "under_score.com",
            "v=spf1 ip4:192.0.2.128/28 -all".to_string(),
        )
        .with_txt(
            "example.com",
            "v=spf1 include:under_score.com -all".to_string(),
        );

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 65]), &resolver).await;
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
async fn ip4() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
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

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt("example.com", "v=spf1 ip4:192.0.2.128 -all".to_string());

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 65]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 128]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'ip4:192.0.2.128/32' directive".to_owned(),
        },
        "{result:?}"
    );
}

#[tokio::test]
async fn ip6() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt(
            "example.com",
            "v=spf1 ip6:2a01:111:f400::/48 -all".to_string(),
        );

    let result = evaluate_ip(
        Ipv6Addr::from([
            0x1a, 0x01, 0x01, 0x11, 0xf4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]),
        &resolver,
    )
    .await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = evaluate_ip(
        Ipv6Addr::from([
            0x2a, 0x01, 0x01, 0x11, 0xf4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]),
        &resolver,
    )
    .await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'ip6:2a01:111:f400::/48' directive".to_owned(),
        },
        "{result:?}"
    );

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .unwrap()
        .with_txt("example.com", "v=spf1 ip6:2a01:111:f400:: -all".to_string());

    let result = evaluate_ip(
        Ipv6Addr::from([
            0x1a, 0x01, 0x01, 0x11, 0xf4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]),
        &resolver,
    )
    .await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = evaluate_ip(
        Ipv6Addr::from([
            0x2a, 0x01, 0x01, 0x11, 0xf4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]),
        &resolver,
    )
    .await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'ip6:2a01:111:f400::/128' directive".to_owned(),
        },
        "{result:?}"
    );
}

// Ensure that a split spf record is joined and parsed correctly
// <https://datatracker.ietf.org/doc/html/rfc7208#section-3.3>
#[tokio::test]
async fn txt_record_joining() {
    let resolver = TestResolver::default()
        .with_zone(
            r#"; https://datatracker.ietf.org/doc/html/rfc7208#section-3.3
$ORIGIN example.com.
@       600 TXT "v=spf1 " "?all"
            TXT "something else"
"#,
        )
        .unwrap();
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
    let resolver = TestResolver::default()
        .with_zone(
            r#"; https://datatracker.ietf.org/doc/html/rfc7208#section-3.3
$ORIGIN example.com.
@       600 TXT "not spf"
"#,
        )
        .unwrap();
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
        .unwrap()
        .with_zone(ADDR_192)
        .unwrap()
        .with_zone(ADDR_10)
        .unwrap()
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

#[tokio::test]
async fn lookup_limits() {
    let mut resolver = TestResolver::default().with_zone(EXAMPLE_COM).unwrap();
    for i in 1..=15 {
        resolver = resolver.with_txt(
            &format!("inc{i}.com"),
            format!("v=spf1 redirect=inc{}.com", i + 1),
        );
    }

    let resolver = resolver.with_txt("example.com", "v=spf1 redirect=inc1.com".to_string());

    let result = evaluate_ip(Ipv4Addr::from([192, 0, 2, 65]), &resolver).await;
    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::PermError,
            context: "DNS lookup limits exceeded".to_owned(),
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

#[tokio::test]
async fn test_exp() {
    let resolver = TestResolver::default()
        .with_txt(
            "explain.example.com",
            "%{i} is not one of %{d}'s designated mail servers. \
            See http://%{d}/why.html?s=%{S}&i=%{I}. Helo was %{h}",
        )
        .with_txt("example.com", "v=spf1 mx -all exp=explain.example.com");

    let cx = SpfContext::new(
        "sender@example.com",
        "example.com",
        Ipv4Addr::LOCALHOST.into(),
    )
    .unwrap()
    .with_ehlo_domain(Some("hi.example.com"))
    .with_relaying_host_name(Some("mx.example.com"));

    let result = cx.check(&resolver, true).await;
    eprintln!("{result:#?}");
    assert_eq!(result.disposition, SpfDisposition::Fail);
    assert_eq!(
        result.context,
        "127.0.0.1 is not one of example.com's \
        designated mail servers. See \
        http://example.com/why.html?s=sender%40example.com&i=127.0.0.1. \
        Helo was hi.example.com"
    );
}

/// This test is a little bit disingenuous, because the issue it is testing
/// is impossible to reproduce with the TestResolver.  The issue was that
/// the underlying resolver would propagate a NoRecordsFound hickory error
/// as a DnsError::ResolveFailed instead of returning an empty list of
/// ip addresses.  The error would essentially blow up the exists: rule which
/// is defined as being OK in the face of having no matching records.
#[tokio::test]
async fn no_records_for_exists_should_not_block_otherwise_satisfied_eval() {
    let resolver = TestResolver::default()
        .with_txt(
            "greenhouse.io",
            "v=spf1 include:_spf.salesforce.com include:mg-spf.greenhouse.io ~all",
        )
        .with_txt(
            "_spf.salesforce.com",
            // Note that we don't provide any of these IP._spf.mta.salesforce.com
            // A or AAAA entries, so the exists checks will all fail
            "v=spf1 exists:%{i}._spf.mta.salesforce.com -all",
        )
        .with_txt(
            "mg-spf.greenhouse.io",
            "v=spf1 ip4:185.250.239.148 ip4:185.250.239.168 ip4:185.250.239.190 \
                ip4:198.244.59.30 ip4:198.244.59.33 ip4:198.244.59.35 \
                ip4:198.61.254.21 ip4:209.61.151.236 ip4:209.61.151.249 \
                ip4:209.61.151.251 ip4:69.72.40.93 ip4:69.72.40.94/31 \
                ip4:69.72.40.96/30 ip4:69.72.47.205 ~all",
        );

    let cx = SpfContext::new(
        "sender@greenhouse.io",
        "greenhouse.io",
        "69.72.47.205".parse().unwrap(),
    )
    .unwrap();
    let result = cx.check(&resolver, true).await;
    eprintln!("{result:#?}");
    assert_eq!(result.disposition, SpfDisposition::Pass);
    assert_eq!(
        result.context,
        "matched 'include:mg-spf.greenhouse.io' directive"
    );
}

/// This is the live-dns version of the above
/// no_records_for_exists_should_not_block_otherwise_satisfied_eval test case
/// that queries real DNS with a real resolver. Prior to the fix for this issue,
/// this test would fail.
#[cfg(feature = "live-dns-tests")]
#[tokio::test]
async fn live_no_records_for_exists_should_not_block_otherwise_satisfied_eval() {
    use dns_resolver::HickoryResolver;
    let resolver = HickoryResolver::new().unwrap();
    let cx = SpfContext::new(
        "sender@greenhouse.io",
        "greenhouse.io",
        "69.72.47.205".parse().unwrap(),
    )
    .unwrap();
    let result = cx.check(&resolver, true).await;
    eprintln!("{result:#?}");
    assert_eq!(result.disposition, SpfDisposition::Pass);
    assert_eq!(
        result.context,
        "matched 'include:mg-spf.greenhouse.io' directive"
    );
}
