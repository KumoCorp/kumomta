use crate::kumod::{DaemonWithMaildir, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_api_types::{AbortReadyQConnV1Request, InspectQueueV1Request, InspectReadyQV1Request};
use kumo_log_types::{JsonLogRecord, RecordType};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use sha2::{Digest, Sha256};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(15);

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
    reason: &str,
) -> anyhow::Result<()> {
    let records = daemon.source.collect_logs().await?;
    let failure = records
        .iter()
        .find(|r| r.kind == RecordType::TransientFailure && r.recipient == [recipient])
        .context("missing expected transient failure")?;
    anyhow::ensure!(
        failure.response.content.contains(reason),
        "unexpected failure for {recipient}: {failure:?}"
    );
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
async fn enforce_cannot_reuse_another_domains_plaintext() -> anyhow::Result<()> {
    let (_ca, options) = fixture("mx.shared.example")?;
    let daemon = run_shared_queue_messages(
        options.env("KUMOD_HIDE_STARTTLS", "true"),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@enforce.example", RecordType::TransientFailure),
        ],
    )
    .await?;
    assert_failure(
        &daemon,
        "second@enforce.example",
        "tls policy is Required but STARTTLS is not advertised",
    )
    .await
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
    assert_failure(
        &daemon,
        "first@enforce.example",
        "tls policy is Required but STARTTLS is not advertised",
    )
    .await
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
    assert_failure(
        &daemon,
        "second@none.example",
        "tls policy is Required but STARTTLS is not advertised",
    )
    .await
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
async fn pkix_cannot_bypass_dane_without_mta_sts() -> anyhow::Result<()> {
    let (_ca, options) = fixture("mx.shared.example")?;
    let daemon = run_shared_queue_messages(
        options
            .env("KUMOD_SHARED_MX_NO_STS", "true")
            .env("KUMOD_SHARED_MX_TLSA", format!("3 0 1 {}", "00".repeat(32))),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@secure.example", RecordType::TransientFailure),
        ],
    )
    .await?;
    assert_failure(&daemon, "second@secure.example", "TLS handshake with").await
}

#[tokio::test]
async fn unsigned_domain_cannot_supply_dane_evidence() -> anyhow::Result<()> {
    let (_ca, options) = fixture("not-the-mx.example")?;
    let daemon = run_shared_queue_messages(
        options.env("KUMOD_SHARED_MX_TLS", "OpportunisticInsecure"),
        [
            ("first@none.example", RecordType::Delivery),
            ("second@secure.example", RecordType::Delivery),
        ],
    )
    .await?;
    let records = receptions(&daemon).await?;
    anyhow::ensure!(
        records.len() == 2 && records[0].session_id != records[1].session_id,
        "DANE must authenticate on a new connection: {records:?}"
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
    assert_failure(&daemon, "second@enforce.example", "TLS handshake with").await
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
    assert_failure(&daemon, "second@secure.example", "TLS handshake with").await?;
    assert_encrypted_deliveries(&daemon, 2).await
}

#[tokio::test]
async fn dane_failure_does_not_mark_shared_tls_broken() -> anyhow::Result<()> {
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
    assert_failure(&daemon, "second@enforce.example", "TLS handshake with").await?;
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
    assert_failure(&daemon, "first@none.example", "TLS handshake with").await?;
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
    let records = daemon.source.collect_logs().await?;
    let failure = records
        .iter()
        .find(|r| {
            r.recipient == ["second@secure.example"] && r.kind == RecordType::TransientFailure
        })
        .context("second message must defer")?;
    anyhow::ensure!(failure.response.code >= 400 && failure.response.code < 500);
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
    assert_failure(&daemon, "second@secure.example", "TLS handshake with").await
}

#[tokio::test]
async fn tlsa_failure_defers_only_the_affected_message() -> anyhow::Result<()> {
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
    assert_failure(&daemon, "second@secure.example", "DANE TLSA lookup").await?;
    assert_reused(&daemon).await
}

async fn queue_mx_change(
    dir: &tempfile::TempDir,
    options: DaemonWithMaildirOptions,
) -> anyhow::Result<DaemonWithMaildir> {
    let control = dir.path().join("dns.json");
    let daemon = options
        .env("KUMOD_SHARED_MX_POLICY_MAX_AGE", "1")
        .env("KUMOD_SHARED_MX_TWO_HOSTS", "true")
        .env("KUMOD_SHARED_MX_HOLD", dir.path().display().to_string())
        .env("KUMOD_SHARED_MX_CONTROL", control.display().to_string())
        .start()
        .await?;
    send(&daemon, "hold@moving.example").await?;
    tokio::time::timeout(WAIT, async {
        while !dir.path().join("entered").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("sink did not hold the first delivery")?;
    send(&daemon, "queued@moving.example").await?;
    send(&daemon, "sibling@sibling.example").await?;
    // Expire the short MTA-STS max_age; TestResolver does not honor zone TTLs.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    std::fs::write(&control, br#"{"allowed_mx":"other.shared.example"}"#)?;
    // This submission installs the new mock policy while the earlier messages
    // remain in the original ready queue behind the blocked delivery.
    send(&daemon, "trigger@none.example").await?;
    std::fs::write(dir.path().join("release"), "")?;
    Ok(daemon)
}

#[tokio::test]
async fn changed_mx_site_reinserts_only_the_affected_message() -> anyhow::Result<()> {
    let (dir, options) = fixture("mx.shared.example")?;
    let mut daemon = queue_mx_change(&dir, options).await?;
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&RecordType::Delivery).copied().unwrap_or(0) == 4,
                WAIT
            )
            .await
    );
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    let records = daemon.source.collect_logs().await?;
    let delivered = |recipient: &str| -> anyhow::Result<&JsonLogRecord> {
        records
            .iter()
            .find(|r| r.kind == RecordType::Delivery && r.recipient == [recipient])
            .with_context(|| format!("missing delivery for {recipient}"))
    };
    let first = delivered("hold@moving.example")?;
    let moved = delivered("queued@moving.example")?;
    let sibling = delivered("sibling@sibling.example")?;
    anyhow::ensure!(first.site != moved.site && first.site == sibling.site);
    anyhow::ensure!(
        moved.num_attempts == 0,
        "rerouting must not consume a delivery attempt"
    );
    anyhow::ensure!(
        moved
            .peer_address
            .as_ref()
            .context("missing peer")?
            .name
            .trim_end_matches('.')
            == "other.shared.example"
    );
    let receptions = receptions(&daemon).await?;
    let first_session = receptions
        .iter()
        .find(|r| r.recipient == ["hold@moving.example"])
        .context("missing first reception")?
        .session_id;
    let sibling_session = receptions
        .iter()
        .find(|r| r.recipient == ["sibling@sibling.example"])
        .context("missing sibling reception")?
        .session_id;
    anyhow::ensure!(
        first_session.is_some() && first_session == sibling_session,
        "rerouting one domain must retain the connection for its siblings"
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum MessageWork {
    Reinsert,
    ReinsertError,
    Defer,
}

async fn abort_message_work(work: MessageWork) -> anyhow::Result<()> {
    let (dir, options) = fixture("mx.shared.example")?;
    let control = dir.path().join("dns.json");
    let options = options
        .env(
            "KUMOD_SHARED_MX_MESSAGE_WORK",
            match work {
                MessageWork::Reinsert => "reinsert",
                MessageWork::ReinsertError => "reinsert-error",
                MessageWork::Defer => "defer",
            },
        )
        .env(
            "KUMOD_SHARED_MX_MESSAGE_WORK_DIR",
            dir.path().display().to_string(),
        )
        .env("KUMOD_SHARED_MX_CONTROL", control.display().to_string());
    let (mut daemon, queue_name, first, victim, deliveries) = match work {
        MessageWork::Reinsert | MessageWork::ReinsertError => (
            queue_mx_change(&dir, options).await?,
            "moving.example",
            "hold@moving.example",
            "queued@moving.example",
            3,
        ),
        MessageWork::Defer => {
            let daemon = options.start().await?;
            send(&daemon, "first@secure.example").await?;
            anyhow::ensure!(daemon.wait_for_maildir_count(1, WAIT).await);
            std::fs::write(&control, br#"{"servfail":true}"#)?;
            send(&daemon, "second@secure.example").await?;
            (
                daemon,
                "secure.example",
                "first@secure.example",
                "second@secure.example",
                1,
            )
        }
    };
    tokio::time::timeout(WAIT, async {
        while !dir.path().join("work-entered").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("message work did not reach the cancellation barrier")?;
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |summary| summary.get(&RecordType::Delivery).copied().unwrap_or(0) > 0,
                WAIT
            )
            .await
    );

    let logs = daemon.source.collect_logs().await?;
    let first = logs
        .iter()
        .find(|r| r.kind == RecordType::Delivery && r.recipient == [first])
        .context("missing first delivery")?;
    let session_id = first.session_id.context("missing dispatcher session")?;
    let api = daemon.source.api_client();
    api.admin_abort_ready_q_conn_v1(&AbortReadyQConnV1Request {
        queue_name: first.site.clone(),
        session_id,
    })
    .await?;
    tokio::time::timeout(WAIT, async {
        loop {
            let state = api
                .admin_inspect_ready_q_v1(&InspectReadyQV1Request {
                    queue_name: first.site.clone(),
                    include_scheduled_queues: false,
                })
                .await?;
            if !state.dispatchers.iter().any(|d| d.session_id == session_id) {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("dispatcher was not aborted")??;

    std::fs::write(dir.path().join("work-release"), "")?;
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let state = api
            .admin_inspect_sched_q_v1(&InspectQueueV1Request {
                queue_name: queue_name.into(),
                want_body: false,
                limit: Some(0),
            })
            .await?;
        anyhow::ensure!(
            state.num_scheduled <= 1,
            "cancellation queued the message more than once: {}",
            state.num_scheduled
        );
        if dir.path().join("work-completed").exists() && state.num_scheduled == 1 {
            break;
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "cancelled dispatcher abandoned message work; scheduled={}",
            state.num_scheduled
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    anyhow::ensure!(daemon.wait_for_maildir_count(deliveries, WAIT).await);
    daemon.source.stop().await?;
    daemon.sink.stop().await?;
    let logs = daemon.source.collect_logs().await?;
    let failures: Vec<_> = logs
        .iter()
        .filter(|r| r.recipient == [victim] && r.kind == RecordType::TransientFailure)
        .collect();
    match work {
        MessageWork::Reinsert => {
            anyhow::ensure!(failures.is_empty(), "rerouting failed: {failures:?}");
            anyhow::ensure!(
                logs.iter().any(|r| r.recipient == [victim]
                    && r.kind == RecordType::Delayed
                    && r.num_attempts == 0),
                "rerouting consumed a delivery attempt"
            );
        }
        MessageWork::ReinsertError => anyhow::ensure!(
            failures.len() == 1
                && failures[0]
                    .response
                    .content
                    .contains("test reinsertion failure"),
            "reinsertion error did not defer the message: {failures:?}"
        ),
        MessageWork::Defer => anyhow::ensure!(
            failures.len() == 1
                && failures[0]
                    .response
                    .content
                    .contains("failed to evaluate message TLS policy"),
            "unexpected policy deferral: {failures:?}"
        ),
    }
    Ok(())
}

#[tokio::test]
async fn cancelled_mx_reinsertion_does_not_duplicate_message() -> anyhow::Result<()> {
    abort_message_work(MessageWork::Reinsert).await
}

#[tokio::test]
async fn cancelled_mx_reinsertion_still_handles_failure() -> anyhow::Result<()> {
    abort_message_work(MessageWork::ReinsertError).await
}

#[tokio::test]
async fn cancelled_policy_deferral_does_not_abandon_message() -> anyhow::Result<()> {
    abort_message_work(MessageWork::Defer).await
}
