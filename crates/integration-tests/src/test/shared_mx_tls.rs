use super::dane::{dane_diagnostics, tls_authentications};
use crate::kumod::{DaemonWithMaildir, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_api_types::InspectReadyQV1Request;
use kumo_log_types::{JsonLogRecord, RecordType};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(15);

const NO_STARTTLS: &str = "400 KumoMTA internal: failed to connect to any candidate hosts: \
    tls policy is Required but STARTTLS is not advertised by ResolvedAddress { \
    name: \"mx.shared.example.\", addr: 127.0.0.1, is_secure: true }:{port}";
const DANE_CERTIFICATE_FAILURE: &str = "400 KumoMTA internal: failed to connect to any \
    candidate hosts: TLS handshake with ResolvedAddress { name: \"mx.shared.example.\", \
    addr: 127.0.0.1, is_secure: true }:{port} failed: certificate verify failed";
const PKIX_NAME_MISMATCH: &str = "400 KumoMTA internal: failed to connect to any candidate \
    hosts: TLS handshake with ResolvedAddress { name: \"mx.shared.example.\", \
    addr: 127.0.0.1, is_secure: true }:{port} failed: invalid peer certificate: \
    certificate not valid for name \"mx.shared.example\"; certificate is only valid \
    for DnsName(\"not-the-mx.example\") or DnsName(\"other.shared.example\")";
const TLSA_LOOKUP_FAILURE: &str =
    "400 KumoMTA internal: failed to connect to any candidate hosts: \
    failed to evaluate message TLS policy: \
    DANE TLSA lookup for mx.shared.example. could not be securely resolved: \
    TLSA lookup for mx.shared.example.:{port} returned Server Failure";

fn fixture(hostname: &str) -> anyhow::Result<(tempfile::TempDir, DaemonWithMaildirOptions)> {
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "Shared MX test CA");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
    ];
    let ca = CertifiedIssuer::self_signed(params, KeyPair::generate()?)?;
    let mut params = CertificateParams::new(vec![
        hostname.to_string(),
        "other.shared.example".to_string(),
    ])?;
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, hostname);
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate()?;
    let cert = params.signed_by(&key, &ca)?;
    let dir = tempfile::tempdir()?;
    let ca_file = dir.path().join("ca.pem");
    std::fs::write(&ca_file, ca.pem())?;
    let options = DaemonWithMaildirOptions::new()
        .policy_file("shared-mx-tls.lua")
        .sink_policy_file("sink-dane.lua")
        .env("SSL_CERT_FILE", ca_file.display().to_string())
        .env("KUMOD_SINK_TLS_CERT", cert.pem())
        .env("KUMOD_SINK_TLS_KEY", key.serialize_pem())
        .env(
            "KUMOD_SHARED_MX_TLSA",
            format!("3 0 1 {}", hex::encode(Sha256::digest(cert.der()))),
        );
    Ok((dir, options))
}

async fn send(daemon: &DaemonWithMaildir, recipient: &str) -> anyhow::Result<()> {
    let mut client = daemon.smtp_client().await?;
    let response = MailGenParams {
        recip: Some(recipient),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250, "submission failed: {response:?}");
    Ok(())
}

async fn run_shared_queue_messages<const N: usize>(
    options: DaemonWithMaildirOptions,
    messages: [(&str, RecordType); N],
) -> anyhow::Result<DaemonWithMaildir> {
    let mut daemon = options.start().await?;
    for (index, (recipient, expected)) in messages.iter().enumerate() {
        send(&daemon, recipient).await?;
        let count = messages[..=index]
            .iter()
            .filter(|(_, kind)| kind == expected)
            .count();
        anyhow::ensure!(
            daemon
                .wait_for_source_summary(
                    |summary| summary.get(expected).copied().unwrap_or(0) >= count,
                    WAIT
                )
                .await,
            "{recipient} did not produce {expected:?}"
        );
    }
    // Do not stop the receiver while the source is still flushing deliveries.
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    let delivered = messages
        .iter()
        .filter(|(_, kind)| *kind == RecordType::Delivery)
        .count();
    anyhow::ensure!(
        daemon.extract_maildir_messages()?.len() == delivered,
        "receiver captures disagree with the expected outcomes"
    );
    let records = daemon.source.collect_logs().await?;
    for (recipient, expected) in messages {
        let dispositions: Vec<_> = records
            .iter()
            .filter(|r| {
                r.recipient == [recipient]
                    && matches!(r.kind, RecordType::Delivery | RecordType::TransientFailure)
            })
            .collect();
        anyhow::ensure!(
            dispositions.len() == 1 && dispositions[0].kind == expected,
            "{recipient}: {dispositions:?}"
        );
    }
    let outcomes: Vec<_> = records
        .iter()
        .filter(|r| matches!(r.kind, RecordType::Delivery | RecordType::TransientFailure))
        .collect();
    let first = outcomes.first().context("no delivery outcomes")?;
    anyhow::ensure!(
        outcomes.iter().all(|record| record.site == first.site),
        "test did not exercise a shared ready queue: {outcomes:?}"
    );
    Ok(daemon)
}

async fn receptions(daemon: &DaemonWithMaildir) -> anyhow::Result<Vec<JsonLogRecord>> {
    Ok(daemon
        .sink
        .collect_logs()
        .await?
        .into_iter()
        .filter(|r| r.kind == RecordType::Reception)
        .collect())
}

async fn assert_failure(
    daemon: &DaemonWithMaildir,
    recipient: &str,
    expected: &str,
) -> anyhow::Result<()> {
    let records = daemon.source.collect_logs().await?;
    let failure = records
        .iter()
        .find(|r| r.kind == RecordType::TransientFailure && r.recipient == [recipient])
        .context("missing expected transient failure")?;
    let response = failure.response.to_single_line().replace(
        &format!(":{}", daemon.sink.listener("smtp").port()),
        ":{port}",
    );
    // Normalize OpenSSL's version-specific error details while preserving
    // the certificate-verification failure.
    let response =
        Regex::new(r"error:[0-9A-Fa-f]+:SSL routines:[^:]*:certificate verify failed:.*")?
            .replace_all(&response, "certificate verify failed");
    k9::assert_equal!(response.as_ref(), expected);
    Ok(())
}

async fn assert_encrypted_deliveries(
    daemon: &DaemonWithMaildir,
    count: usize,
) -> anyhow::Result<()> {
    let records = receptions(daemon).await?;
    anyhow::ensure!(
        records.len() == count && records.iter().all(|r| r.tls_cipher.is_some()),
        "expected {count} encrypted deliveries: {records:?}"
    );
    Ok(())
}

async fn assert_reused(daemon: &DaemonWithMaildir) -> anyhow::Result<()> {
    assert_encrypted_deliveries(daemon, 2).await?;
    let records = receptions(daemon).await?;
    anyhow::ensure!(
        records.len() == 2
            && records[0].session_id.is_some()
            && records[0].session_id == records[1].session_id,
        "expected reuse of the actual receiver connection: {records:?}"
    );
    Ok(())
}

#[tokio::test]
async fn none_does_not_inherit_another_domains_enforcement() -> anyhow::Result<()> {
    let (_ca, options) = fixture("mx.shared.example")?;
    let daemon = run_shared_queue_messages(
        options.env("KUMOD_HIDE_STARTTLS", "true"),
        [
            ("first@enforce.example", RecordType::TransientFailure),
            ("second@none.example", RecordType::Delivery),
        ],
    )
    .await?;
    assert_failure(&daemon, "first@enforce.example", NO_STARTTLS).await
}

#[tokio::test]
async fn routing_domain_enforcement_overrides_envelope_domain() -> anyhow::Result<()> {
    let (_ca, options) = fixture("mx.shared.example")?;
    let daemon = run_shared_queue_messages(
        options
            .env("KUMOD_HIDE_STARTTLS", "true")
            .env("KUMOD_SHARED_MX_ROUTING_DOMAIN", "enforce.example"),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@none.example", RecordType::TransientFailure),
        ],
    )
    .await?;
    assert_failure(&daemon, "second@none.example", NO_STARTTLS).await
}

#[tokio::test]
async fn envelope_enforcement_does_not_override_routing_domain() -> anyhow::Result<()> {
    let (_ca, options) = fixture("mx.shared.example")?;
    run_shared_queue_messages(
        options
            .env("KUMOD_HIDE_STARTTLS", "true")
            .env("KUMOD_SHARED_MX_ROUTING_DOMAIN", "none.example"),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@enforce.example", RecordType::Delivery),
        ],
    )
    .await?;
    Ok(())
}

enum TlsBackend {
    #[cfg(target_os = "linux")]
    Rustls,
    OpenSsl,
}

async fn reuse_pkix(backend: TlsBackend) -> anyhow::Result<()> {
    let (_ca, options) = fixture("mx.shared.example")?;
    let prefer_openssl = match backend {
        #[cfg(target_os = "linux")]
        TlsBackend::Rustls => "false",
        TlsBackend::OpenSsl => "true",
    };
    let daemon = run_shared_queue_messages(
        options.env("KUMOD_SHARED_MX_OPENSSL", prefer_openssl),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@enforce.example", RecordType::Delivery),
        ],
    )
    .await?;
    assert_reused(&daemon).await
}

// The rustls fixture relies on Linux's SSL_CERT_FILE trust override.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn pkix_reused_across_domains_rustls() -> anyhow::Result<()> {
    reuse_pkix(TlsBackend::Rustls).await
}

#[tokio::test]
async fn pkix_reused_across_domains_openssl() -> anyhow::Result<()> {
    reuse_pkix(TlsBackend::OpenSsl).await
}

#[tokio::test]
async fn unsigned_domain_cannot_supply_dane_evidence() -> anyhow::Result<()> {
    let (_ca, options) = fixture("not-the-mx.example")?;
    let mut daemon = options
        .env("KUMOD_SHARED_MX_TLS", "OpportunisticInsecure")
        .start()
        .await?;
    let tracer = daemon.source.trace_client().await?;
    for (index, recipient) in ["first@none.example", "second@secure.example"]
        .iter()
        .enumerate()
    {
        send(&daemon, recipient).await?;
        anyhow::ensure!(
            daemon
                .wait_for_source_summary(
                    |s| s.get(&RecordType::Delivery).copied().unwrap_or(0) > index,
                    WAIT,
                )
                .await,
            "{recipient} was not delivered"
        );
    }
    anyhow::ensure!(
        tracer
            .wait_for(|events| tls_authentications(events).len() == 2, WAIT)
            .await,
        "missing TLS handshake diagnostics"
    );
    let events = tracer.stop().await?;
    k9::assert_equal!(tls_authentications(&events), vec![false, true]);
    k9::assert_equal!(
        dane_diagnostics(&events),
        vec![
            "DANE is enabled but the chain to mx.shared.example. is not fully DNSSEC-secure \
             (mx_selection_secure=false, address_secure=true); not using DANE",
            "DANE records for mx.shared.example. are: [<tlsa records>]",
        ]
    );
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    let logs = daemon.source.collect_logs().await?;
    let delivered: Vec<_> = logs
        .iter()
        .filter(|r| r.kind == RecordType::Delivery)
        .collect();
    anyhow::ensure!(
        delivered.len() == 2 && delivered[0].site == delivered[1].site,
        "test did not exercise a shared ready queue: {delivered:?}"
    );
    k9::assert_equal!(daemon.extract_maildir_messages()?.len(), 2);
    assert_encrypted_deliveries(&daemon, 2).await?;
    let records = receptions(&daemon).await?;
    anyhow::ensure!(
        records.iter().all(|r| r.session_id.is_some())
            && records[0].session_id != records[1].session_id,
        "DANE must authenticate on a new connection: {records:?}"
    );
    Ok(())
}

#[tokio::test]
async fn enforce_cannot_reuse_unverified_tls_with_dane_site_names() -> anyhow::Result<()> {
    let (_ca, options) = fixture("not-the-mx.example")?;
    let daemon = run_shared_queue_messages(
        options
            .env("KUMOD_SHARED_MX_TLS", "OpportunisticInsecure")
            .env("KUMOD_SHARED_MX_DANE_SITE_NAMES", "true"),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@enforce.example", RecordType::TransientFailure),
        ],
    )
    .await?;
    assert_failure(&daemon, "second@enforce.example", PKIX_NAME_MISMATCH).await?;
    let records = daemon.source.collect_logs().await?;
    anyhow::ensure!(
        records
            .iter()
            .filter(|r| matches!(r.kind, RecordType::Delivery | RecordType::TransientFailure))
            .all(|r| r.site.contains("[dane-insecure]")),
        "the test must exercise the DANE-isolated ready queue"
    );
    Ok(())
}

#[tokio::test]
async fn dane_site_names_keep_secure_and_insecure_queues_separate() -> anyhow::Result<()> {
    let (_ca, options) = fixture("not-the-mx.example")?;
    let mut daemon = options
        .env("KUMOD_SHARED_MX_TLS", "OpportunisticInsecure")
        .env("KUMOD_SHARED_MX_DANE_SITE_NAMES", "true")
        .start()
        .await?;
    for (index, recipient) in ["first@none.example", "second@secure.example"]
        .iter()
        .enumerate()
    {
        send(&daemon, recipient).await?;
        anyhow::ensure!(
            daemon
                .wait_for_source_summary(
                    |s| s.get(&RecordType::Delivery).copied().unwrap_or(0) > index,
                    WAIT,
                )
                .await,
            "{recipient} was not delivered"
        );
    }
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    let records = daemon.source.collect_logs().await?;
    let delivered: Vec<_> = records
        .iter()
        .filter(|r| r.kind == RecordType::Delivery)
        .collect();
    anyhow::ensure!(delivered.len() == 2, "{delivered:?}");
    anyhow::ensure!(
        delivered[0].site.contains("[dane-insecure]")
            && !delivered[1].site.contains("[dane-insecure]")
            && delivered[0].site != delivered[1].site,
        "expected separate insecure and secure ready queues: {delivered:?}"
    );
    assert_encrypted_deliveries(&daemon, 2).await?;
    let received = receptions(&daemon).await?;
    anyhow::ensure!(
        received[0].session_id.is_some() && received[0].session_id != received[1].session_id,
        "separate ready queues must use separate connections: {received:?}"
    );
    Ok(())
}

#[tokio::test]
async fn dane_is_not_pkix_evidence() -> anyhow::Result<()> {
    let (_ca, options) = fixture("not-the-mx.example")?;
    let daemon = run_shared_queue_messages(
        options,
        [
            ("first@secure.example", RecordType::Delivery),
            ("second@enforce.example", RecordType::TransientFailure),
        ],
    )
    .await?;
    assert_failure(&daemon, "second@enforce.example", PKIX_NAME_MISMATCH).await
}

#[tokio::test]
async fn dane_host_proof_reused_across_secure_domains() -> anyhow::Result<()> {
    let (_ca, options) = fixture("not-the-mx.example")?;
    let daemon = run_shared_queue_messages(
        options,
        [
            ("first@secure.example", RecordType::Delivery),
            ("second@secure2.example", RecordType::Delivery),
        ],
    )
    .await?;
    assert_reused(&daemon).await
}

enum BrokenTlsMemory {
    Configured,
    ReconnectOption,
}

impl BrokenTlsMemory {
    fn enable(self, options: DaemonWithMaildirOptions) -> DaemonWithMaildirOptions {
        match self {
            Self::Configured => options.env("KUMOD_SHARED_MX_REMEMBER_BROKEN_TLS", "1h"),
            Self::ReconnectOption => options.env("KUMOD_SHARED_MX_TLS_RECONNECT", "true"),
        }
    }
}

async fn dane_failure_keeps_sibling_tls(memory: BrokenTlsMemory) -> anyhow::Result<()> {
    let (_ca, options) = fixture("mx.shared.example")?;
    // The configured-memory case also checks DANE activation without MTA-STS.
    let options = if matches!(memory, BrokenTlsMemory::Configured) {
        options.env("KUMOD_SHARED_MX_NO_STS", "true")
    } else {
        options
    };
    let daemon = run_shared_queue_messages(
        memory
            .enable(options)
            .env("KUMOD_SHARED_MX_TLSA", format!("3 0 1 {}", "00".repeat(32))),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@secure.example", RecordType::TransientFailure),
            ("third@none.example", RecordType::Delivery),
        ],
    )
    .await?;
    assert_failure(&daemon, "second@secure.example", DANE_CERTIFICATE_FAILURE).await?;
    assert_encrypted_deliveries(&daemon, 2).await
}

#[tokio::test]
async fn dane_without_mta_sts_failure_does_not_mark_shared_tls_broken() -> anyhow::Result<()> {
    dane_failure_keeps_sibling_tls(BrokenTlsMemory::Configured).await
}

#[tokio::test]
async fn dane_failure_does_not_enable_reconnect_memory() -> anyhow::Result<()> {
    dane_failure_keeps_sibling_tls(BrokenTlsMemory::ReconnectOption).await
}

#[tokio::test]
async fn mta_sts_enforce_failure_does_not_mark_shared_tls_broken() -> anyhow::Result<()> {
    let (_ca, options) = fixture("not-the-mx.example")?;
    let daemon = run_shared_queue_messages(
        BrokenTlsMemory::Configured
            .enable(options)
            .env("KUMOD_SHARED_MX_TLS", "OpportunisticInsecure"),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@enforce.example", RecordType::TransientFailure),
            ("third@none.example", RecordType::Delivery),
        ],
    )
    .await?;
    assert_failure(&daemon, "second@enforce.example", PKIX_NAME_MISMATCH).await?;
    assert_encrypted_deliveries(&daemon, 2).await
}

#[tokio::test]
async fn opportunistic_failure_still_marks_shared_tls_broken() -> anyhow::Result<()> {
    let (_ca, options) = fixture("not-the-mx.example")?;
    let daemon = run_shared_queue_messages(
        BrokenTlsMemory::Configured.enable(options),
        [
            ("first@none.example", RecordType::TransientFailure),
            ("second@none.example", RecordType::Delivery),
        ],
    )
    .await?;
    assert_failure(&daemon, "first@none.example", PKIX_NAME_MISMATCH).await?;
    let records = receptions(&daemon).await?;
    anyhow::ensure!(
        records.len() == 1 && records[0].tls_cipher.is_none(),
        "opportunistic failure did not retain plaintext fallback: {records:?}"
    );
    Ok(())
}

async fn refreshed_dane_failure(
    changes: serde_json::Value,
) -> anyhow::Result<(tempfile::TempDir, DaemonWithMaildir)> {
    let (dir, options) = fixture("not-the-mx.example")?;
    let control = dir.path().join("dns.json");
    let daemon = options
        .env("KUMOD_SHARED_MX_CONTROL", control.display().to_string())
        // An unsigned sibling can reuse encrypted DANE without requiring PKIX.
        .env("KUMOD_SHARED_MX_TLS", "RequiredInsecure")
        .start()
        .await?;
    send(&daemon, "first@secure.example").await?;
    anyhow::ensure!(daemon.wait_for_maildir_count(1, WAIT).await);
    std::fs::write(&control, serde_json::to_vec(&changes)?)?;
    send(&daemon, "second@secure.example").await?;
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&RecordType::TransientFailure).copied().unwrap_or(0) > 0,
                WAIT
            )
            .await
    );
    anyhow::ensure!(
        daemon.extract_maildir_messages()?.len() == 1,
        "the second message bypassed DANE"
    );
    Ok((dir, daemon))
}

#[tokio::test]
async fn changed_tlsa_requires_new_authentication() -> anyhow::Result<()> {
    let (_dir, mut daemon) =
        refreshed_dane_failure(serde_json::json!({"tlsa": format!("3 0 1 {}", "00".repeat(32))}))
            .await?;
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    assert_failure(&daemon, "second@secure.example", DANE_CERTIFICATE_FAILURE).await
}

#[tokio::test]
async fn tlsa_failure_uses_normal_connection_retry() -> anyhow::Result<()> {
    let (_dir, mut daemon) = refreshed_dane_failure(serde_json::json!({"servfail": true})).await?;
    send(&daemon, "third@none.example").await?;
    anyhow::ensure!(
        daemon.wait_for_maildir_count(2, WAIT).await,
        "healthy sibling was deferred"
    );
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    let records = receptions(&daemon).await?;
    anyhow::ensure!(records
        .iter()
        .any(|r| r.recipient == ["third@none.example"]));
    assert_failure(&daemon, "second@secure.example", TLSA_LOOKUP_FAILURE).await?;
    assert_encrypted_deliveries(&daemon, 2).await?;
    anyhow::ensure!(
        records.iter().all(|r| r.session_id.is_some())
            && records[0].session_id != records[1].session_id,
        "failed policy evaluation must end the connection attempt: {records:?}"
    );
    Ok(())
}

#[tokio::test]
async fn tlsa_failure_tries_the_next_mx() -> anyhow::Result<()> {
    let (dir, options) = fixture("not-the-mx.example")?;
    let control = dir.path().join("dns.json");
    let mut daemon = options
        .env("KUMOD_SHARED_MX_CONTROL", control.display().to_string())
        .env("KUMOD_SHARED_MX_TWO_HOSTS", "true")
        .env("KUMOD_SHARED_MX_TLS", "RequiredInsecure")
        .start()
        .await?;
    send(&daemon, "first@secure.example").await?;
    anyhow::ensure!(daemon.wait_for_maildir_count(1, WAIT).await);
    std::fs::write(&control, br#"{"servfail":true}"#)?;
    send(&daemon, "second@secure.example").await?;
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&RecordType::Delivery).copied().unwrap_or(0) == 2,
                WAIT,
            )
            .await,
        "TLSA failure did not advance to the next MX"
    );
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    let records = daemon.source.collect_logs().await?;
    let delivered: Vec<_> = records
        .iter()
        .filter(|r| r.kind == RecordType::Delivery)
        .collect();
    k9::assert_equal!(delivered.len(), 2);
    k9::assert_equal!(
        delivered[0]
            .peer_address
            .as_ref()
            .context("missing first peer")?
            .name
            .as_str(),
        "mx.shared.example."
    );
    k9::assert_equal!(
        delivered[1]
            .peer_address
            .as_ref()
            .context("missing second peer")?
            .name
            .as_str(),
        "other.shared.example."
    );
    anyhow::ensure!(
        !records
            .iter()
            .any(|r| r.kind == RecordType::TransientFailure),
        "an available alternate MX should avoid deferral"
    );
    assert_encrypted_deliveries(&daemon, 2).await
}

enum SmtpPortSource {
    EgressPath,
    RoutingDomain,
}

#[tokio::test]
async fn queued_messages_retain_their_admission_context() -> anyhow::Result<()> {
    retained_admission_context(SmtpPortSource::EgressPath).await
}

#[tokio::test]
async fn port_qualified_routes_retain_their_admission_context() -> anyhow::Result<()> {
    retained_admission_context(SmtpPortSource::RoutingDomain).await
}

async fn retained_admission_context(port_source: SmtpPortSource) -> anyhow::Result<()> {
    let (dir, options) = fixture("mx.shared.example")?;
    let control = dir.path().join("dns.json");
    let mut daemon = options
        .env("KUMOD_HIDE_STARTTLS", "true")
        .env("KUMOD_SHARED_MX_TWO_HOSTS", "true")
        .env("KUMOD_SHARED_MX_HOLD", dir.path().display().to_string())
        .env("KUMOD_SHARED_MX_CONTROL", control.display().to_string())
        .env(
            "KUMOD_SHARED_MX_PORT_QUALIFIED_ROUTING",
            match port_source {
                SmtpPortSource::EgressPath => "false",
                SmtpPortSource::RoutingDomain => "true",
            },
        )
        .start()
        .await?;
    let (port_suffix, no_starttls) = match port_source {
        SmtpPortSource::EgressPath => (String::new(), NO_STARTTLS.to_string()),
        SmtpPortSource::RoutingDomain => (
            format!(":{}", daemon.sink.listener("smtp").port()),
            NO_STARTTLS.replace("addr: 127.0.0.1,", "addr: 127.0.0.1:{port},"),
        ),
    };
    let shared_queue = format!("unspecified->(mx|other).shared.example{port_suffix}@smtp_client");
    let later_queue = format!("unspecified->other.shared.example{port_suffix}@smtp_client");
    send(&daemon, "hold@none.example").await?;
    tokio::time::timeout(WAIT, async {
        while !dir.path().join("entered").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("sink did not hold the first delivery")?;
    send(&daemon, "queued@moving.example").await?;
    send(&daemon, "sibling@sibling.example").await?;
    let api = daemon.source.api_client();
    tokio::time::timeout(WAIT, async {
        loop {
            let state = api
                .admin_inspect_ready_q_v1(&InspectReadyQV1Request {
                    queue_name: shared_queue.clone(),
                    include_scheduled_queues: false,
                })
                .await?;
            if state.state.ready_count == 2 {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("messages did not reach the ready queue")??;

    // Expire moving.example's one-second MX/policy cache entry. Messages
    // already admitted must retain the old context; later admissions refresh.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    std::fs::write(&control, br#"{"allowed_mx":"other.shared.example"}"#)?;
    send(&daemon, "trigger@none.example").await?;
    send(&daemon, "later@moving.example").await?;
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&RecordType::TransientFailure).copied().unwrap_or(0) > 0,
                WAIT,
            )
            .await,
        "new admission did not enforce the updated policy"
    );
    std::fs::write(dir.path().join("release"), "")?;
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&RecordType::Delivery).copied().unwrap_or(0) == 4,
                WAIT,
            )
            .await,
        "queued messages did not retain their admission context"
    );
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    let records = daemon.source.collect_logs().await?;
    let queued = records
        .iter()
        .find(|r| r.kind == RecordType::Delivery && r.recipient == ["queued@moving.example"])
        .context("missing queued delivery")?;
    let later = records
        .iter()
        .find(|r| r.kind == RecordType::TransientFailure && r.recipient == ["later@moving.example"])
        .context("missing later deferral")?;
    k9::assert_equal!(queued.site.as_str(), shared_queue.as_str());
    k9::assert_equal!(later.site.as_str(), later_queue.as_str());
    k9::assert_equal!(queued.num_attempts, 0);
    assert_failure(
        &daemon,
        "later@moving.example",
        &no_starttls.replace("mx.shared.example", "other.shared.example"),
    )
    .await?;
    let received = receptions(&daemon).await?;
    k9::assert_equal!(received.len(), 4);
    k9::assert_equal!(daemon.extract_maildir_messages()?.len(), 4);
    let first_session = received[0].session_id.context("missing receiver session")?;
    anyhow::ensure!(
        received
            .iter()
            .all(|r| r.tls_cipher.is_none() && r.session_id == Some(first_session)),
        "admitted messages must keep the original plaintext connection: {received:?}"
    );
    Ok(())
}

#[tokio::test]
async fn retry_uses_new_admission_context() -> anyhow::Result<()> {
    let (dir, options) = fixture("mx.shared.example")?;
    let control = dir.path().join("dns.json");
    std::fs::write(&control, br#"{"allowed_mx":"mx.shared.example"}"#)?;
    let mut daemon = options
        .env("KUMOD_HIDE_STARTTLS", "true")
        .env("KUMOD_SHARED_MX_TWO_HOSTS", "true")
        .env("KUMOD_SHARED_MX_RETRY_INTERVAL", "1s")
        .env("KUMOD_SHARED_MX_CONTROL", control.display().to_string())
        .start()
        .await?;
    send(&daemon, "retry@moving.example").await?;
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&RecordType::TransientFailure).copied().unwrap_or(0) > 0,
                WAIT,
            )
            .await,
        "initial enforce policy did not defer delivery"
    );
    std::fs::write(&control, b"{}")?;
    send(&daemon, "trigger@none.example").await?;
    anyhow::ensure!(
        daemon.wait_for_maildir_count(2, WAIT).await,
        "retry retained the previous ready-queue context"
    );
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    assert_failure(&daemon, "retry@moving.example", NO_STARTTLS).await?;
    let records = daemon.source.collect_logs().await?;
    let delivered: Vec<_> = records
        .iter()
        .filter(|r| r.kind == RecordType::Delivery && r.recipient == ["retry@moving.example"])
        .collect();
    k9::assert_equal!(delivered.len(), 1);
    k9::assert_equal!(
        delivered[0].site.as_str(),
        "unspecified->(mx|other).shared.example@smtp_client"
    );
    anyhow::ensure!(delivered[0].num_attempts > 0 && delivered[0].tls_cipher.is_none());
    Ok(())
}
