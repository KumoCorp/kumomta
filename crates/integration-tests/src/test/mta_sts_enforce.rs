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

/// Under an MTA-STS testing policy, `Opportunistic` becomes
/// `OpportunisticInsecure`. Required and disabled modes, including DANE's
/// mandatory STARTTLS, are left unchanged.
#[tokio::test]
async fn mta_sts_testing_tls_modes() -> anyhow::Result<()> {
    for (tls, hide_starttls, unusable_dane, expected, encrypted) in [
        ("Required", true, false, TransientFailure, false),
        ("Required", false, false, TransientFailure, false),
        ("RequiredInsecure", true, false, TransientFailure, false),
        ("RequiredInsecure", false, false, Delivery, true),
        ("Opportunistic", true, false, Delivery, false),
        ("Opportunistic", false, false, Delivery, true),
        ("OpportunisticInsecure", false, false, Delivery, true),
        ("Disabled", false, false, Delivery, false),
        // The testing policy must not relax DANE's TLS requirement.
        ("Disabled", true, true, TransientFailure, false),
    ] {
        // With broken-TLS memory enabled, the opportunistic cases must still
        // use TLS when advertised, despite the sink's untrusted certificate.
        let mut options = DaemonWithMaildirOptions::new()
            .policy_file("mta-sts.lua")
            .env("KUMOD_TESTING_TLS", tls)
            .env("KUMOD_TESTING_REMEMBER_BROKEN_TLS", "3 days");
        if hide_starttls {
            options = options.env("KUMOD_HIDE_STARTTLS", "1");
        }
        if unusable_dane {
            options = options.env("KUMOD_TESTING_DANE_UNUSABLE", "1");
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
                    Duration::from_secs(10),
                )
                .await,
            "no result for {tls}"
        );
        daemon.stop_both().await?;
        let records = daemon.source.collect_logs().await?;
        let outcome = records
            .iter()
            .find(|r| matches!(r.kind, Delivery | TransientFailure))
            .context("testing-policy disposition")?;
        anyhow::ensure!(
            outcome.kind == expected,
            "{tls}, hide_starttls={hide_starttls}, unusable_dane={unusable_dane}: {outcome:?}"
        );
        if expected == Delivery {
            anyhow::ensure!(
                outcome.tls_cipher.is_some() == encrypted,
                "{tls}: {outcome:?}"
            );
            anyhow::ensure!(daemon.extract_maildir_messages()?.len() == 1);
        } else {
            let expected_error = if hide_starttls {
                let policy = if unusable_dane {
                    "RequiredInsecure"
                } else {
                    tls
                };
                format!("tls policy is {policy} but STARTTLS is not advertised")
            } else {
                "invalid peer certificate".to_string()
            };
            anyhow::ensure!(
                outcome.response.content.contains(&expected_error),
                "{tls}, unusable_dane={unusable_dane}: expected {expected_error:?}, got {outcome:?}"
            );
            anyhow::ensure!(daemon.extract_maildir_messages()?.is_empty());
        }
    }
    Ok(())
}
