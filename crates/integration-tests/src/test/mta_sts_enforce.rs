use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

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

/// The expected outcome of delivery under the fixture's MTA-STS policy.
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
    run_mta_sts_policy_case("recipient@testing.example.com", tls, starttls, expect).await
}

/// The fixture's policy hook proves routing/TLS behavior, not policy-cache
/// retention or HTTPS authentication. The mta-sts crate separately exercises
/// the real cache in cached_enforce_survives_mx_and_txt_nodata.
async fn run_mta_sts_policy_case(
    recip: &str,
    tls: &str,
    starttls: StartTlsEhlo,
    expect: Expect,
) -> anyhow::Result<()> {
    let mut options = DaemonWithMaildirOptions::new()
        .policy_file("mta-sts.lua")
        .env("KUMOD_TESTING_TLS", tls)
        .env("KUMOD_ENABLE_MTA_STS", "1");
    if let StartTlsEhlo::Hidden = starttls {
        options = options.env("KUMOD_HIDE_STARTTLS", "1");
    }
    let mut daemon = options.start().await.context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams {
        recip: Some(recip),
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
            anyhow::ensure!(outcome.response.is_transient(), "{outcome:?}");
            anyhow::ensure!(
                !records.iter().any(|r| r.kind == Delivery),
                "unexpected source delivery: {records:?}"
            );
            let sink_records = daemon.sink.collect_logs().await?;
            anyhow::ensure!(
                !sink_records
                    .iter()
                    .any(|r| r.kind == kumo_log_types::RecordType::Reception),
                "sink accepted a message: {sink_records:?}"
            );
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

/// NODATA selects an implicit MX, but does not exempt that host from enforce.
#[tokio::test]
async fn mta_sts_implicit_mx_rejects_disallowed_host() -> anyhow::Result<()> {
    run_mta_sts_policy_case(
        "recipient@implicit-denied.example.com",
        "OpportunisticInsecure",
        StartTlsEhlo::Hidden,
        Expect::Deferred(
            "failed to resolve queue implicit-denied.example.com: MTA-STS enforce \
             policy for implicit-denied.example.com permits none of its MX hosts \
             [\"implicit-denied.example.com.\"]; allowed mx patterns: [\"allowed.example.net\"]. \
             The destination is undeliverable until its MTA-STS policy is corrected.",
        ),
    )
    .await
}

/// A permitted implicit MX must offer STARTTLS even with an opportunistic floor.
#[tokio::test]
async fn mta_sts_implicit_mx_requires_starttls() -> anyhow::Result<()> {
    run_mta_sts_policy_case(
        "recipient@implicit-enforce.example.com",
        "OpportunisticInsecure",
        StartTlsEhlo::Hidden,
        Expect::Deferred(
            "KumoMTA internal: failed to connect to any candidate hosts: tls policy is \
             Required but STARTTLS is not advertised by ResolvedAddress { name: \
             \"implicit-enforce.example.com.\", addr: 127.0.0.1, is_secure: false }:{port}",
        ),
    )
    .await
}

/// Enforce also raises an insecure floor to authenticated TLS for implicit MX.
#[tokio::test]
async fn mta_sts_implicit_mx_rejects_untrusted_cert() -> anyhow::Result<()> {
    run_mta_sts_policy_case(
        "recipient@implicit-enforce.example.com",
        "OpportunisticInsecure",
        StartTlsEhlo::Advertised,
        Expect::Deferred(
            "KumoMTA internal: failed to connect to any candidate hosts: TLS handshake \
             with ResolvedAddress { name: \"implicit-enforce.example.com.\", addr: 127.0.0.1, \
             is_secure: false }:{port} failed: invalid peer certificate: UnknownIssuer",
        ),
    )
    .await
}

/// Legitimate no-policy NODATA remains deliverable using the configured TLS floor.
#[tokio::test]
async fn mta_sts_implicit_mx_without_policy_delivers_plaintext() -> anyhow::Result<()> {
    run_mta_sts_policy_case(
        "recipient@implicit-nopolicy.example.com",
        "OpportunisticInsecure",
        StartTlsEhlo::Hidden,
        Expect::DeliveredInPlaintext,
    )
    .await
}

/// Testing mode neither blocks a nonmatching implicit host nor requires TLS.
#[tokio::test]
async fn mta_sts_implicit_mx_testing_delivers_plaintext() -> anyhow::Result<()> {
    run_mta_sts_policy_case(
        "recipient@implicit-testing.example.com",
        "OpportunisticInsecure",
        StartTlsEhlo::Hidden,
        Expect::DeliveredInPlaintext,
    )
    .await
}

/// Address literals are direct routes, not implicit MX policy domains.
#[tokio::test]
async fn mta_sts_address_literal_delivers_plaintext() -> anyhow::Result<()> {
    run_mta_sts_policy_case(
        "recipient@[127.0.0.1]",
        "OpportunisticInsecure",
        StartTlsEhlo::Hidden,
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
