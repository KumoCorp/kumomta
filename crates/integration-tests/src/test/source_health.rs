use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::Delivery;
use std::time::Duration;

/// Send a single message through a source whose `source_address` is
/// unplumbed and whose `suspend_when_unplumbed` rule is `Immediate`.
/// The first failed bind must auto-suspend the source, which we observe
/// via the `egress_source_health_suspended` gauge.
#[tokio::test]
async fn source_health_unplumbed_suspends() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-health-unplumbed.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    // The Immediate rule must promote the first BindError into a suspension.
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| {
                m.name().as_str() == "egress_source_health_suspended"
                    && m.label_is("reason", "Unplumbed")
            },
            |values| values.iter().any(|v| *v == 1.0),
        )
        .await
        .context("egress_source_health_suspended{reason=Unplumbed} should reach 1")?;

    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| m.name().as_str() == "egress_source_health_suspensions_total",
            |values| values.iter().any(|v| *v >= 1.0),
        )
        .await
        .context("egress_source_health_suspensions_total should be >= 1")?;

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}

/// Same shape but the failure is a proxy connect failure rather than a
/// local unplumbed bind. The `suspend_when_proxy_unhealthy` rule must
/// fire and the gauge must indicate the source is suspended with
/// `reason=ProxyUnhealthy`.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn source_health_proxy_suspends() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-health-broken-proxy.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| {
                m.name().as_str() == "egress_source_health_suspended"
                    && m.label_is("reason", "ProxyUnhealthy")
            },
            |values| values.iter().any(|v| *v == 1.0),
        )
        .await
        .context("egress_source_health_suspended{reason=ProxyUnhealthy} should reach 1")?;

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}

/// A pool contains one healthy and one unplumbed source. Mail must
/// continue to flow through the healthy source after the unplumbed
/// one auto-suspends. Every delivery in the source-side log must
/// record `egress_source = "good"`, and the bad source's gauge must
/// be set to 1.
#[tokio::test]
async fn source_health_other_pool_member_still_delivers() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-health-mixed-pool.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    // Send enough messages to be confident that round-robin selection
    // routes at least one through the bad source, triggering suspension.
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    for _ in 0..10 {
        let response = MailGenParams::default().send(&mut client).await?;
        anyhow::ensure!(response.code == 250);
    }

    daemon
        .wait_for_maildir_count(10, Duration::from_secs(30))
        .await;

    // Confirm the bad source was auto-suspended (proves the rule fired,
    // which can only happen if selection routed at least one attempt
    // through `bad` before failover).
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| {
                m.name().as_str() == "egress_source_health_suspended" && m.label_is("source", "bad")
            },
            |values| values.iter().any(|v| *v == 1.0),
        )
        .await
        .context("bad source should be auto-suspended")?;

    // Wait for every message to be recorded as a successful delivery in
    // the source log before we stop the daemon and read it back.
    let delivered = daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 10,
            Duration::from_secs(30),
        )
        .await;
    anyhow::ensure!(delivered, "expected 10 Delivery records in the source log");

    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    let delivered_sources: Vec<&str> = logs
        .iter()
        .filter(|r| r.kind == Delivery)
        .filter_map(|r| r.egress_source.as_deref())
        .collect();

    anyhow::ensure!(
        delivered_sources.len() >= 10,
        "expected at least 10 Delivery records, got {} ({delivered_sources:?})",
        delivered_sources.len()
    );
    anyhow::ensure!(
        delivered_sources.iter().all(|s| *s == "good"),
        "every delivery must go via the healthy source, got {delivered_sources:?}"
    );

    Ok(())
}
