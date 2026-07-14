use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use kumo_api_types::{TraceSmtpClientV1Event, TraceSmtpClientV1Payload};
use kumo_log_types::RecordType::{self, TransientFailure};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::time::Duration;

/// Generate a self-signed certificate for the sink to present, returning the
/// certificate PEM, the private key PEM, and the hex-encoded SHA-256 digest of
/// the full certificate (the association data for a `3 0 1` TLSA record).
fn make_sink_cert() -> anyhow::Result<(String, String, String)> {
    let mut params = CertificateParams::new(vec!["mx.dane.example".to_string()])?;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "mx.dane.example");
    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    let digest = Sha256::digest(cert.der());
    Ok((cert.pem(), key.serialize_pem(), hex::encode(digest)))
}

/// The TLSA record the sink publishes for the MX host. The cert digest is only
/// known once the per-run sink certificate is generated, so the concrete record
/// is built inside `run_scenario` from these intent-level variants.
enum Tlsa {
    /// No TLSA record published (used with `servfail` to model a failed lookup).
    None,
    /// A usable `3 0 1` record whose digest matches the sink certificate, so
    /// DANE authenticates the peer.
    MatchesCert,
    /// A usable `3 0 1` record whose digest does not match, so DANE rejects the
    /// presented certificate.
    WrongDigest,
    /// A `1 0 1` PKIX-EE record: published but unusable for SMTP DANE, so
    /// STARTTLS is required without authenticating the peer.
    PkixEeUnusable,
}

/// Inputs that define a single DANE delivery scenario. The `Default` is the
/// happy path (secure zone, matching TLSA, DNS-MX routing); each test overrides
/// only the field(s) that express what it is exercising.
struct Scenario {
    /// Whether the MX, address, and TLSA records are served from a
    /// DNSSEC-secure zone.
    secure: bool,
    /// The TLSA record the sink publishes for the MX host.
    tlsa: Tlsa,
    /// Force the TLSA lookup to return SERVFAIL.
    servfail: bool,
    /// Make the MX host a CNAME into a separate, unsigned zone, so its address
    /// records are insecure while the MX and TLSA records stay in the secure
    /// zone (RFC 7672 section 2.2.2).
    cname_into_unsigned: bool,
    /// Recipient address, which selects DNS-MX (`dane.example`) vs local
    /// `mx_list` (`mxlist.example`) routing in source-dane.lua.
    recipient: &'static str,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            secure: true,
            tlsa: Tlsa::MatchesCert,
            servfail: false,
            cname_into_unsigned: false,
            recipient: "recip@dane.example",
        }
    }
}

/// Whether the attempt is expected to deliver, or to fail transiently (defer).
#[derive(PartialEq)]
enum ExpectDelivery {
    True,
    False,
}

/// A compact, stable view of how a delivery attempt concluded, suitable for
/// snapshotting. The `dane_diagnostics` come from the outbound session tracer
/// and are the authoritative signal for how DANE was applied (independent of
/// the TLS backend), so a regression in DANE handling shows up here.
#[derive(Debug)]
#[allow(dead_code)]
struct Outcome {
    kind: RecordType,
    response: String,
    dane_diagnostics: Vec<String>,
    /// Whether the TLS peer certificate was authenticated, per the client
    /// session tracer. `None` means no TLS session was established (e.g. we
    /// deferred before connecting, or the handshake failed). This is the
    /// RFC 7672 4.1 distinction: usable records require an authenticated peer,
    /// whereas the all-unusable case requires only encryption.
    tls_authenticated: Option<bool>,
}

/// Remove run-to-run variable detail (the sink's ephemeral port, message ids,
/// the published TLSA records, and OpenSSL's version-specific error suffix) so
/// responses and diagnostics can be snapshotted.
fn sanitize(text: &str) -> String {
    let text = Regex::new(r":\d{2,5}\b")
        .unwrap()
        .replace_all(text, ":PORT");
    let text = Regex::new(r"ids=[0-9a-f]+")
        .unwrap()
        .replace_all(&text, "ids=ID");
    let text = Regex::new(r"are: \[.*")
        .unwrap()
        .replace_all(&text, "are: [<tlsa records>]");
    Regex::new(r"error:[0-9A-Fa-f]+:.*")
        .unwrap()
        .replace_all(&text, "<certificate verify failed>")
        .into_owned()
}

fn dane_diagnostics(events: &[TraceSmtpClientV1Event]) -> Vec<String> {
    let mut result: Vec<String> = vec![];
    for event in events {
        if let TraceSmtpClientV1Payload::Diagnostic { message, .. } = &event.payload {
            if message.contains("DANE") {
                let message = sanitize(message);
                if !result.contains(&message) {
                    result.push(message);
                }
            }
        }
    }
    result
}

fn tls_authenticated(events: &[TraceSmtpClientV1Event]) -> Option<bool> {
    let re = Regex::new(r"authenticated: (true|false)").unwrap();
    for event in events {
        if let TraceSmtpClientV1Payload::Diagnostic { message, .. } = &event.payload {
            if message.starts_with("STARTTLS handshake ->") {
                if let Some(caps) = re.captures(message) {
                    return Some(&caps[1] == "true");
                }
            }
        }
    }
    None
}

async fn run_scenario(scenario: Scenario, expect: ExpectDelivery) -> anyhow::Result<Outcome> {
    let (cert_pem, key_pem, digest) = make_sink_cert()?;
    let expect_delivery = expect == ExpectDelivery::True;

    let tlsa = match scenario.tlsa {
        Tlsa::None => None,
        Tlsa::MatchesCert => Some(format!("3 0 1 {digest}")),
        Tlsa::WrongDigest => Some(format!("3 0 1 {}", "00".repeat(32))),
        Tlsa::PkixEeUnusable => Some(format!("1 0 1 {digest}")),
    };

    let mut options = DaemonWithMaildirOptions::new()
        .policy_file("source-dane.lua")
        .sink_policy_file("sink-dane.lua")
        .env("KUMOD_SINK_TLS_CERT", cert_pem)
        .env("KUMOD_SINK_TLS_KEY", key_pem)
        .env(
            "KUMOD_DANE_SECURE",
            if scenario.secure { "true" } else { "false" },
        );
    if let Some(tlsa) = tlsa {
        options = options.env("KUMOD_DANE_TLSA", tlsa);
    }
    if scenario.servfail {
        options = options.env("KUMOD_DANE_SERVFAIL", "true");
    }
    if scenario.cname_into_unsigned {
        options = options.env("KUMOD_DANE_CNAME_UNSIGNED", "true");
    }

    let mut daemon = options.start().await?;

    // Attach the outbound session tracer before triggering delivery; diagnostics
    // are only broadcast while a tracer is subscribed.
    let tracer = daemon.source.trace_client().await?;

    let mut client = daemon.smtp_client().await?;
    let response = MailGenParams {
        recip: Some(scenario.recipient),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    if expect_delivery {
        daemon
            .wait_for_maildir_count(1, Duration::from_secs(30))
            .await;
    } else {
        daemon
            .wait_for_source_summary(
                |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
                Duration::from_secs(30),
            )
            .await;
    }

    tracer
        .wait_for(
            |events| {
                !dane_diagnostics(events).is_empty()
                    && (!expect_delivery || tls_authenticated(events).is_some())
            },
            Duration::from_secs(5),
        )
        .await;
    let events = tracer.stop().await?;

    daemon.stop_both().await?;

    let records = daemon.source.collect_logs().await?;
    let record = records
        .iter()
        .rev()
        .find(|record| {
            matches!(
                record.kind,
                RecordType::Delivery | RecordType::TransientFailure | RecordType::Bounce
            )
        })
        .ok_or_else(|| anyhow::anyhow!("no terminal record found in {records:#?}"))?;

    Ok(Outcome {
        kind: record.kind,
        response: sanitize(&record.response.to_single_line()),
        dane_diagnostics: dane_diagnostics(&events),
        tls_authenticated: tls_authenticated(&events),
    })
}

#[tokio::test]
async fn dane_authenticated() -> anyhow::Result<()> {
    let outcome = run_scenario(Scenario::default(), ExpectDelivery::True).await?;
    k9::snapshot!(
        outcome,
        r#"
Outcome {
    kind: Delivery,
    response: "250 OK ids=ID",
    dane_diagnostics: [
        "DANE records for mx.dane.example. are: [<tlsa records>]",
    ],
    tls_authenticated: Some(
        true,
    ),
}
"#
    );
    Ok(())
}

#[tokio::test]
async fn dane_mx_list_authenticated() -> anyhow::Result<()> {
    // A locally-configured mx_list host with treat_mx_list_as_secure = true is
    // a trusted selection, so DANE applies and authenticates the peer.
    let outcome = run_scenario(
        Scenario {
            recipient: "recip@mxlist.example",
            ..Default::default()
        },
        ExpectDelivery::True,
    )
    .await?;
    k9::snapshot!(
        outcome,
        r#"
Outcome {
    kind: Delivery,
    response: "250 OK ids=ID",
    dane_diagnostics: [
        "DANE records for mx.dane.example are: [<tlsa records>]",
    ],
    tls_authenticated: Some(
        true,
    ),
}
"#
    );
    Ok(())
}

#[tokio::test]
async fn dane_mx_list_untrusted_is_opportunistic() -> anyhow::Result<()> {
    // An mx_list route without treat_mx_list_as_secure: the selection is not
    // known to be DNSSEC-secure, so DANE must not engage even though the host's
    // address and TLSA records are in a secure zone. Delivery falls back to
    // opportunistic (unauthenticated) TLS.
    let outcome = run_scenario(
        Scenario {
            recipient: "recip@mxlistinsecure.example",
            ..Default::default()
        },
        ExpectDelivery::True,
    )
    .await?;
    k9::snapshot!(
        outcome,
        r#"
Outcome {
    kind: Delivery,
    response: "250 OK ids=ID",
    dane_diagnostics: [
        "DANE is enabled but the chain to mx.dane.example is not fully DNSSEC-secure (mx_selection_secure=false, address_secure=true); not using DANE",
    ],
    tls_authenticated: Some(
        false,
    ),
}
"#
    );
    Ok(())
}

#[tokio::test]
async fn dane_secure_cname_into_unsigned_authenticated() -> anyhow::Result<()> {
    // The MX host is a securely published CNAME whose target lands in an
    // unsigned zone, so its address records are insecure. RFC 7672 section
    // 2.2.2 keeps the host DANE-eligible at the original MX name; DANE engages
    // via the secure alias and authenticates the peer.
    let outcome = run_scenario(
        Scenario {
            cname_into_unsigned: true,
            ..Default::default()
        },
        ExpectDelivery::True,
    )
    .await?;
    k9::snapshot!(
        outcome,
        r#"
Outcome {
    kind: Delivery,
    response: "250 OK ids=ID",
    dane_diagnostics: [
        "mx.dane.example. resolves via a secure CNAME into an insecure zone; DANE remains eligible at the original name (RFC 7672 section 2.2.2)",
        "DANE records for mx.dane.example. are: [<tlsa records>]",
    ],
    tls_authenticated: Some(
        true,
    ),
}
"#
    );
    Ok(())
}

#[tokio::test]
async fn dane_mismatch_defers() -> anyhow::Result<()> {
    // DANE records were used (see the diagnostic), but the TLSA digest does not
    // match the presented certificate, so the DANE verification rejects it
    // (opportunistic-insecure would have accepted the same self-signed cert).
    let outcome = run_scenario(
        Scenario {
            tlsa: Tlsa::WrongDigest,
            ..Default::default()
        },
        ExpectDelivery::False,
    )
    .await?;
    k9::snapshot!(
        outcome,
        r#"
Outcome {
    kind: TransientFailure,
    response: "400 KumoMTA internal: failed to connect to any candidate hosts: TLS handshake with ResolvedAddress { name: "mx.dane.example.", addr: 127.0.0.1, is_secure: true }:PORT failed: <certificate verify failed>",
    dane_diagnostics: [
        "DANE records for mx.dane.example. are: [<tlsa records>]",
    ],
    tls_authenticated: None,
}
"#
    );
    Ok(())
}

#[tokio::test]
async fn dane_insecure_is_opportunistic() -> anyhow::Result<()> {
    let outcome = run_scenario(
        Scenario {
            secure: false,
            ..Default::default()
        },
        ExpectDelivery::True,
    )
    .await?;
    k9::snapshot!(
        outcome,
        r#"
Outcome {
    kind: Delivery,
    response: "250 OK ids=ID",
    dane_diagnostics: [
        "DANE is enabled but the chain to mx.dane.example. is not fully DNSSEC-secure (mx_selection_secure=false, address_secure=false); not using DANE",
    ],
    tls_authenticated: Some(
        false,
    ),
}
"#
    );
    Ok(())
}

#[tokio::test]
async fn dane_unusable_requires_unauthenticated_tls() -> anyhow::Result<()> {
    let outcome = run_scenario(
        Scenario {
            tlsa: Tlsa::PkixEeUnusable,
            ..Default::default()
        },
        ExpectDelivery::True,
    )
    .await?;
    k9::snapshot!(
        outcome,
        r#"
Outcome {
    kind: Delivery,
    response: "250 OK ids=ID",
    dane_diagnostics: [
        "DANE TLSA records for mx.dane.example. exist but none are usable; requiring unauthenticated STARTTLS",
    ],
    tls_authenticated: Some(
        false,
    ),
}
"#
    );
    Ok(())
}

#[tokio::test]
async fn dane_servfail_defers() -> anyhow::Result<()> {
    let outcome = run_scenario(
        Scenario {
            tlsa: Tlsa::None,
            servfail: true,
            ..Default::default()
        },
        ExpectDelivery::False,
    )
    .await?;
    k9::snapshot!(
        outcome,
        r#"
Outcome {
    kind: TransientFailure,
    response: "400 KumoMTA internal: failed to connect to any candidate hosts: DANE TLSA lookup for mx.dane.example. could not be securely resolved: TLSA lookup for mx.dane.example.:PORT returned Server Failure",
    dane_diagnostics: [
        "DANE TLSA lookup for mx.dane.example. could not be securely resolved: TLSA lookup for mx.dane.example.:PORT returned Server Failure",
    ],
    tls_authenticated: None,
}
"#
    );
    Ok(())
}
