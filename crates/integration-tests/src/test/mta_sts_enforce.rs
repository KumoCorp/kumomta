use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_api_types::{InspectReadyQV1Request, TraceSmtpClientV1Payload};
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::path::Path;
use std::time::{Duration, Instant};

/// End-to-end coverage for the MTA-STS aliasing fix (#484): a domain whose
/// enforce-mode policy permits none of its MX hosts must fail to resolve with a
/// transient error, rather than silently affecting other domains that share
/// its MX hosts.
#[tokio::test]
async fn mta_sts_enforce_impossible() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("mta-sts.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        recip: Some("victim@broken.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let records = daemon.source.collect_logs().await?;
    let failure = records
        .iter()
        .find(|r| r.kind == TransientFailure)
        .context("expected a TransientFailure record")?;

    let normalized = mod_smtp_response_normalize::normalize(&failure.response.to_single_line());
    k9::snapshot!(
        normalized,
        r#"451 4.4.4 failed to resolve queue broken.example.com: MTA-STS enforce policy for broken.example.com permits none of its MX hosts "mail.broken.example.com." allowed mx patterns: "allowed.example.net" The destination is undeliverable until its MTA-STS policy is corrected."#
    );

    Ok(())
}

/// Companion to [`mta_sts_enforce_impossible`]: an enforce-mode policy whose
/// allowed MX patterns cover the domain's MX host leaves resolution unchanged,
/// so delivery proceeds normally.
#[tokio::test]
async fn mta_sts_enforce_match() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("mta-sts.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        recip: Some("winner@good.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );

    Ok(())
}

/// Whether the sink advertises STARTTLS in its EHLO response.
enum StartTlsEhlo {
    Advertised,
    Hidden,
}

/// The expected outcome of delivery to the MTA-STS testing domain.
enum Expect {
    DeliveredOverTls,
    DeliveredInPlaintext,
    /// Deferred with this exact response content, where the resolved peer's
    /// ephemeral port is written as the literal `{port}`.
    Deferred(&'static str),
}

/// Rewrites the resolved peer's ephemeral port in `content` to the literal
/// `{port}`. The port is the only part of a deferral response that changes
/// between runs.
fn mask_peer_port(content: &str) -> String {
    let marker = "is_secure: false }:";
    let Some(idx) = content.find(marker) else {
        return content.to_string();
    };
    let port_start = idx + marker.len();
    let rest = &content[port_start..];
    let port_end = port_start
        + rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
    format!("{}{{port}}{}", &content[..port_start], &content[port_end..])
}

/// Sends a message to testing.example.com, which publishes an MTA-STS testing
/// policy, through an egress path configured with `enable_tls = tls`, and
/// checks the outcome of its first delivery attempt.
async fn run_testing_policy_case(
    tls: &str,
    starttls: StartTlsEhlo,
    expect: Expect,
) -> anyhow::Result<()> {
    let mut options = DaemonWithMaildirOptions::new()
        .policy_file("mta-sts.lua")
        .env("KUMOD_TESTING_TLS", tls);
    if let StartTlsEhlo::Hidden = starttls {
        options = options.env("KUMOD_HIDE_STARTTLS", "1");
    }
    let mut daemon = options.start().await.context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams {
        recip: Some("recipient@testing.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250, "{response:?}");

    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&Delivery).copied().unwrap_or(0) > 0
                    || s.get(&TransientFailure).copied().unwrap_or(0) > 0,
                Duration::from_secs(50),
            )
            .await,
        "no delivery outcome"
    );
    daemon.stop_both().await.context("stop_both")?;

    let records = daemon.source.collect_logs().await?;
    let outcome = records
        .iter()
        .find(|r| matches!(r.kind, Delivery | TransientFailure))
        .context("delivery outcome")?;
    let delivered = daemon.extract_maildir_messages()?.len();

    match expect {
        Expect::DeliveredOverTls | Expect::DeliveredInPlaintext => {
            let over_tls = matches!(expect, Expect::DeliveredOverTls);
            anyhow::ensure!(
                outcome.kind == Delivery && outcome.tls_cipher.is_some() == over_tls,
                "expected delivery with over_tls={over_tls}, got {outcome:?}"
            );
            anyhow::ensure!(delivered == 1, "{delivered} messages in the maildir");
        }
        Expect::Deferred(expected) => {
            anyhow::ensure!(
                outcome.kind == TransientFailure,
                "expected a deferral, got {outcome:?}"
            );
            k9::assert_equal!(mask_peer_port(&outcome.response.content).as_str(), expected);
            anyhow::ensure!(delivered == 0, "{delivered} messages in the maildir");
        }
    }
    Ok(())
}

/// A testing policy does not lower `Opportunistic`: an untrusted certificate
/// still fails validation and defers, rather than being delivered over
/// unauthenticated TLS.
#[tokio::test]
async fn mta_sts_testing_opportunistic_defers_untrusted_cert() -> anyhow::Result<()> {
    run_testing_policy_case(
        "Opportunistic",
        StartTlsEhlo::Advertised,
        Expect::Deferred(
            "KumoMTA internal: failed to connect to any candidate hosts: TLS handshake \
             with ResolvedAddress { name: \"mail.testing.example.com.\", addr: 127.0.0.1, \
             is_secure: false }:{port} failed: invalid peer certificate: UnknownIssuer",
        ),
    )
    .await
}

/// Checks that `OpportunisticInsecure`, which already opts out of certificate
/// validation, still delivers over TLS to the untrusted certificate when a
/// testing policy is present.
#[tokio::test]
async fn mta_sts_testing_opportunistic_insecure_delivers_over_tls() -> anyhow::Result<()> {
    run_testing_policy_case(
        "OpportunisticInsecure",
        StartTlsEhlo::Advertised,
        Expect::DeliveredOverTls,
    )
    .await
}

/// Checks that a testing policy does not relax `Required` to plaintext delivery
/// when the MX does not offer STARTTLS.
#[tokio::test]
async fn mta_sts_testing_required_without_starttls() -> anyhow::Result<()> {
    run_testing_policy_case(
        "Required",
        StartTlsEhlo::Hidden,
        Expect::Deferred(
            "KumoMTA internal: failed to connect to any candidate hosts: tls policy is \
             Required but STARTTLS is not advertised by ResolvedAddress { name: \
             \"mail.testing.example.com.\", addr: 127.0.0.1, is_secure: false }:{port}",
        ),
    )
    .await
}

/// A testing policy does not skip certificate validation for `Required`.
#[tokio::test]
async fn mta_sts_testing_required_untrusted_cert() -> anyhow::Result<()> {
    run_testing_policy_case(
        "Required",
        StartTlsEhlo::Advertised,
        Expect::Deferred(
            "KumoMTA internal: failed to connect to any candidate hosts: TLS handshake \
             with ResolvedAddress { name: \"mail.testing.example.com.\", addr: 127.0.0.1, \
             is_secure: false }:{port} failed: invalid peer certificate: UnknownIssuer",
        ),
    )
    .await
}

/// A testing policy does not relax `RequiredInsecure`'s mandatory STARTTLS.
#[tokio::test]
async fn mta_sts_testing_required_insecure_without_starttls() -> anyhow::Result<()> {
    run_testing_policy_case(
        "RequiredInsecure",
        StartTlsEhlo::Hidden,
        Expect::Deferred(
            "KumoMTA internal: failed to connect to any candidate hosts: tls policy is \
             RequiredInsecure but STARTTLS is not advertised by ResolvedAddress { name: \
             \"mail.testing.example.com.\", addr: 127.0.0.1, is_secure: false }:{port}",
        ),
    )
    .await
}

/// A testing policy does not enable STARTTLS when TLS is `Disabled`.
#[tokio::test]
async fn mta_sts_testing_disabled_delivers_plaintext() -> anyhow::Result<()> {
    run_testing_policy_case(
        "Disabled",
        StartTlsEhlo::Advertised,
        Expect::DeliveredInPlaintext,
    )
    .await
}

/// Verifies that when a domain lacking an MTA-STS policy
/// (`nopolicy.example.com`) and one that publishes enforce
/// (`enforce.example.com`) share a ready queue, the no-policy domain opens a
/// cleartext connection and the dispatcher defers the enforce domain's message
/// rather than sending it over that connection.
#[tokio::test]
async fn mta_sts_cleartext_veto_defers_enforce_domain() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("mta-sts-shared-queue.lua")
        .env("KUMOD_HIDE_STARTTLS", "1")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    // nopolicy.example.com lacks an MTA-STS policy. KUMOD_HIDE_STARTTLS keeps
    // the sink from advertising STARTTLS, and Opportunistic TLS only attempts a
    // handshake when the peer advertises it, leaving this delivery in plaintext.
    let response = MailGenParams {
        recip: Some("user@nopolicy.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send to nopolicy.example.com")?;
    anyhow::ensure!(response.code == 250, "{response:?}");
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&Delivery).copied().unwrap_or(0) > 0,
                Duration::from_secs(50),
            )
            .await,
        "nopolicy.example.com was not delivered"
    );

    // The 250 here only confirms spool acceptance. Kumod applies
    // enforce.example.com's policy later, at the outbound attempt over the
    // shared connection.
    let response = MailGenParams {
        recip: Some("user@enforce.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send to enforce.example.com")?;
    anyhow::ensure!(response.code == 250, "{response:?}");
    // The fixture lowers retry_interval from the 20-minute default to 2s;
    // with the exponential backoff (2s, 4s, 8s, ...) several deferrals fit
    // in this 50s wait.
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&TransientFailure).copied().unwrap_or(0) >= 2,
                Duration::from_secs(50),
            )
            .await,
        "enforce.example.com was not deferred twice"
    );

    daemon.stop_both().await.context("stop_both")?;

    let records = daemon.source.collect_logs().await?;

    // nopolicy.example.com is the only domain that delivered.
    let delivered_recipients: Vec<&str> = records
        .iter()
        .filter(|r| r.kind == Delivery)
        .flat_map(|r| r.recipient.iter().map(String::as_str))
        .collect();
    k9::assert_equal!(delivered_recipients, vec!["user@nopolicy.example.com"]);

    let deferrals = records
        .iter()
        .filter(|r| r.kind == TransientFailure)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        deferrals.len() >= 2,
        "expected at least two deferrals, got {}: {records:#?}",
        deferrals.len()
    );
    for deferral in &deferrals {
        k9::assert_equal!(
            deferral.recipient,
            vec!["user@enforce.example.com".to_string()]
        );
        k9::assert_equal!(deferral.response.code, 451);
        k9::assert_equal!(
            deferral.response.content.as_str(),
            "KumoMTA internal: enforce.example.com publishes an MTA-STS enforce \
            policy requiring TLS, but the connection to \
            unspecified->shared.example.com@smtp_client is cleartext; \
            deferring rather than delivering without TLS"
        );
    }

    // Exactly one message reached the sink: nopolicy.example.com's.
    k9::assert_equal!(daemon.extract_maildir_messages()?.len(), 1);

    Ok(())
}

#[derive(Clone, Copy)]
enum CachedStsPolicy {
    Enforce,
    Testing,
    None,
    Absent,
    ExpiredEnforce,
}

async fn wait_for_marker(path: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.try_exists()? {
        anyhow::ensure!(Instant::now() < deadline, "waiting for {}", path.display());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

/// The policy hook simulates fetching an authenticated body and fills the real
/// MTA-STS cache. Production fetch/cache semantics are tested in the mta-sts
/// crate; this test checks promotion, MX expiry/SERVFAIL, and dispatcher reuse.
async fn run_cached_policy_mx_failure(policy: CachedStsPolicy) -> anyhow::Result<()> {
    let mode = match policy {
        CachedStsPolicy::Enforce | CachedStsPolicy::ExpiredEnforce => "enforce",
        CachedStsPolicy::Testing => "testing",
        CachedStsPolicy::None => "none",
        CachedStsPolicy::Absent => "absent",
    };
    let mut options = DaemonWithMaildirOptions::new()
        .policy_file("mta-sts-veto-mx-failure.lua")
        .env("KUMOD_TEST_STS_MODE", mode)
        .env("KUMOD_HIDE_STARTTLS", "1")
        // The source deliberately holds a connection while the test observes
        // promotion. Do not let the sink's normal 3s idle timeout race it.
        .env("KUMOD_TEST_SINK_CLIENT_TIMEOUT", "60s");
    if matches!(policy, CachedStsPolicy::ExpiredEnforce) {
        options = options.env("KUMOD_TEST_STS_MAX_AGE", "2");
    }
    let mut daemon = options.start().await.context("start MX-failure fixture")?;
    let tracer = daemon.source.trace_client().await?;
    let mut client = daemon.smtp_client().await?;

    // The sink rejects this recipient without closing the connection. The
    // source's rewrite-status hook holds the dispatcher before it logs the
    // result, allowing a sibling to be promoted onto the same ready queue.
    let response = MailGenParams {
        recip: Some("450-hold@nopolicy.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250, "{response:?}");
    wait_for_marker(&daemon.source.dir.path().join("dispatcher-held")).await?;

    let response = MailGenParams {
        recip: Some("user@target.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250, "{response:?}");

    let deadline = Instant::now() + Duration::from_secs(20);
    let session_id = loop {
        let queue = daemon
            .source
            .api_client()
            .admin_inspect_ready_q_v1(&InspectReadyQV1Request {
                queue_name: "unspecified->shared.example.com@smtp_client".to_string(),
                include_scheduled_queues: true,
            })
            .await?;
        if queue.state.ready_count == 1 {
            anyhow::ensure!(queue.state.connection_count == 1, "{queue:?}");
            anyhow::ensure!(queue.dispatchers.len() == 1, "{queue:?}");
            anyhow::ensure!(
                queue
                    .scheduled_queue_names
                    .as_ref()
                    .is_some_and(|names| names.iter().any(|name| name == "target.example.com")),
                "target was not promoted: {queue:?}"
            );
            break queue.dispatchers[0].session_id;
        }
        anyhow::ensure!(Instant::now() < deadline, "target was not ready: {queue:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let promotion: serde_json::Value = serde_json::from_slice(&std::fs::read(
        daemon.source.dir.path().join("promotion-mx.json"),
    )?)?;
    let expected_mode = match policy {
        CachedStsPolicy::Enforce | CachedStsPolicy::ExpiredEnforce => "Enforce",
        CachedStsPolicy::Testing => "Testing",
        CachedStsPolicy::None | CachedStsPolicy::Absent => "None",
    };
    anyhow::ensure!(promotion["mta_sts"] == expected_mode, "{promotion}");
    anyhow::ensure!(
        promotion["domain_name"] == "target.example.com.",
        "{promotion}"
    );
    anyhow::ensure!(
        promotion["hosts"] == serde_json::json!(["shared.example.com."]),
        "{promotion}"
    );

    if matches!(policy, CachedStsPolicy::Enforce) {
        // The target is the only fixture policy. Check that it really entered
        // the production cache, not just that the test hook returned Enforce.
        // Its 86400s lifetime far exceeds this test's bounded barriers.
        let sizes = daemon
            .source
            .api_client()
            .get_metrics(|m| {
                (m.name().as_str() == "lruttl_cache_size"
                    && m.label_is("cache_name", "mta_sts_policy"))
                .then(|| m.value())
            })
            .await?;
        anyhow::ensure!(sizes == vec![1.0], "policy was not cached: {sizes:?}");
    }

    if matches!(policy, CachedStsPolicy::ExpiredEnforce) {
        // This negative control must outlive the 2s policy, not just its 1s
        // MX TTL. No new promotion/policy fetch runs while we hold the sender.
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    std::fs::write(daemon.source.dir.path().join("release-dispatcher"), "")?;
    wait_for_marker(&daemon.source.dir.path().join("mx-failed")).await?;
    let mx_failure = std::fs::read_to_string(daemon.source.dir.path().join("mx-failed"))?;
    anyhow::ensure!(mx_failure.contains("Server Failure"), "{mx_failure}");

    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&Delivery).copied().unwrap_or(0) > 0
                    || s.get(&TransientFailure).copied().unwrap_or(0) >= 2,
                Duration::from_secs(30),
            )
            .await,
        "target has no delivery outcome"
    );
    let events = tracer.stop().await?;
    daemon.stop_both().await?;
    let records = daemon.source.collect_logs().await?;
    let creator = records
        .iter()
        .find(|r| {
            r.kind == TransientFailure
                && r.recipient
                    .iter()
                    .any(|r| r == "450-hold@nopolicy.example.com")
        })
        .context("creator's RCPT rejection")?;
    anyhow::ensure!(creator.response.code == 450, "{creator:?}");
    anyhow::ensure!(creator.tls_cipher.is_none(), "{creator:?}");
    anyhow::ensure!(creator.session_id == Some(session_id), "{creator:?}");
    let target = records
        .iter()
        .find(|r| {
            matches!(r.kind, Delivery | TransientFailure)
                && r.recipient.iter().any(|r| r == "user@target.example.com")
        })
        .context("target outcome")?;
    anyhow::ensure!(
        target.session_id == creator.session_id,
        "dispatcher was not reused: {target:?}"
    );
    // A dispatcher keeps its session_id when it replaces its TCP connection.
    anyhow::ensure!(
        events
            .iter()
            .filter(|event| event.payload == TraceSmtpClientV1Payload::Connected)
            .count()
            == 1,
        "expected reuse of the original TCP connection: {events:#?}"
    );
    anyhow::ensure!(target.tls_cipher.is_none(), "{target:?}");

    if matches!(policy, CachedStsPolicy::Enforce) {
        anyhow::ensure!(
            target.kind == TransientFailure,
            "cached enforce was bypassed: {target:?}"
        );
        k9::assert_equal!(target.response.code, 451);
        k9::assert_equal!(
            target.response.content.as_str(),
            "KumoMTA internal: target.example.com publishes an MTA-STS enforce \
             policy requiring TLS, but the connection to \
             unspecified->shared.example.com@smtp_client is cleartext; \
             deferring rather than delivering without TLS"
        );
        anyhow::ensure!(daemon.extract_maildir_messages()?.is_empty());
    } else {
        anyhow::ensure!(target.kind == Delivery, "unexpected veto: {target:?}");
        k9::assert_equal!(daemon.extract_maildir_messages()?.len(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn mta_sts_mx_failure_retains_cached_enforce() -> anyhow::Result<()> {
    run_cached_policy_mx_failure(CachedStsPolicy::Enforce).await
}

#[tokio::test]
async fn mta_sts_mx_failure_without_cached_policy_allows_plaintext() -> anyhow::Result<()> {
    run_cached_policy_mx_failure(CachedStsPolicy::Absent).await
}

#[tokio::test]
async fn mta_sts_mx_failure_with_cached_testing_allows_plaintext() -> anyhow::Result<()> {
    run_cached_policy_mx_failure(CachedStsPolicy::Testing).await
}

#[tokio::test]
async fn mta_sts_mx_failure_with_cached_none_allows_plaintext() -> anyhow::Result<()> {
    run_cached_policy_mx_failure(CachedStsPolicy::None).await
}

#[tokio::test]
async fn mta_sts_mx_failure_with_expired_enforce_allows_plaintext() -> anyhow::Result<()> {
    run_cached_policy_mx_failure(CachedStsPolicy::ExpiredEnforce).await
}
