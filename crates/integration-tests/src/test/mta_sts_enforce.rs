use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

/// End-to-end coverage for the MTA-STS aliasing fix (#484): a domain whose
/// enforce-mode policy permits none of its MX hosts must fail to resolve with a
/// transient error, rather than silently affecting co-sited siblings. The
/// policy is supplied via `kumo.dns.configure_test_mta_sts`, so no live DNS or
/// HTTPS policy endpoint is involved.
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

/// How delivery to the MTA-STS testing domain is expected to conclude.
enum Expect {
    DeliveredOverTls,
    DeliveredInPlaintext,
    /// Deferred with a response containing this text.
    Deferred(&'static str),
}

/// Sends a message to testing.example.com, which publishes an MTA-STS testing
/// policy, through an egress path configured with `enable_tls = tls`, and
/// checks the outcome of its first delivery attempt. The sink presents an
/// untrusted certificate.
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
    let mut daemon = options.start().await?;
    let mut client = daemon.smtp_client().await?;
    let response = MailGenParams {
        recip: Some("recipient@testing.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250, "{response:?}");
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&Delivery).copied().unwrap_or(0) > 0
                    || s.get(&TransientFailure).copied().unwrap_or(0) > 0,
                Duration::from_secs(30),
            )
            .await,
        "no delivery outcome"
    );
    daemon.stop_both().await?;
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
        Expect::Deferred(reason) => {
            anyhow::ensure!(
                outcome.kind == TransientFailure && outcome.response.content.contains(reason),
                "expected deferral with {reason:?}, got {outcome:?}"
            );
            anyhow::ensure!(delivered == 0, "{delivered} messages in the maildir");
        }
    }
    Ok(())
}

/// A testing policy doesn't relax `Required` to plaintext delivery.
#[tokio::test]
async fn mta_sts_testing_required_without_starttls() -> anyhow::Result<()> {
    run_testing_policy_case(
        "Required",
        StartTlsEhlo::Hidden,
        Expect::Deferred("tls policy is Required but STARTTLS is not advertised"),
    )
    .await
}

/// A testing policy doesn't skip certificate validation for `Required`.
#[tokio::test]
async fn mta_sts_testing_required_untrusted_certificate() -> anyhow::Result<()> {
    run_testing_policy_case(
        "Required",
        StartTlsEhlo::Advertised,
        Expect::Deferred("invalid peer certificate"),
    )
    .await
}

/// A testing policy doesn't relax `RequiredInsecure` to plaintext delivery.
#[tokio::test]
async fn mta_sts_testing_required_insecure_without_starttls() -> anyhow::Result<()> {
    run_testing_policy_case(
        "RequiredInsecure",
        StartTlsEhlo::Hidden,
        Expect::Deferred("tls policy is RequiredInsecure but STARTTLS is not advertised"),
    )
    .await
}

/// A testing policy doesn't enable STARTTLS when TLS is `Disabled`.
#[tokio::test]
async fn mta_sts_testing_disabled() -> anyhow::Result<()> {
    run_testing_policy_case(
        "Disabled",
        StartTlsEhlo::Advertised,
        Expect::DeliveredInPlaintext,
    )
    .await
}

/// A testing policy turns `Opportunistic` into `OpportunisticInsecure`, so the
/// untrusted certificate doesn't prevent TLS delivery.
#[tokio::test]
async fn mta_sts_testing_opportunistic_untrusted_certificate() -> anyhow::Result<()> {
    run_testing_policy_case(
        "Opportunistic",
        StartTlsEhlo::Advertised,
        Expect::DeliveredOverTls,
    )
    .await
}
