use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::{Delivery, Reception};
use std::time::Duration;

#[tokio::test]
async fn mx_site_names_share_exact_destinations_not_label_combinations() -> anyhow::Result<()> {
    // Maildir polling may outlast the sink's default idle timeout. Keep the
    // connection alive for the reuse assertion; the test stops both daemons
    // explicitly rather than waiting for this timeout.
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("mx-site-names.lua")
        .env("KUMOD_TEST_SINK_CLIENT_TIMEOUT", "1m")
        .start()
        .await?;
    let mut client = daemon.smtp_client().await?;
    let recipients = [
        "one@route-a.example",
        "two@route-b.example",
        "three@route-c.example",
    ];
    for (index, recipient) in recipients.iter().enumerate() {
        let response = MailGenParams {
            recip: Some(recipient),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250, "{response:?}");
        anyhow::ensure!(
            daemon
                .wait_for_maildir_count(index + 1, Duration::from_secs(10))
                .await
        );
    }
    daemon.stop_both().await?;
    let source = daemon.source.collect_logs().await?;
    let sink = daemon.sink.collect_logs().await?;
    let mut deliveries = vec![];
    let mut receptions = vec![];
    for recipient in recipients {
        deliveries.push(
            source
                .iter()
                .find(|r| r.kind == Delivery && r.recipient.iter().any(|addr| addr == recipient))
                .context("source delivery")?,
        );
        receptions.push(
            sink.iter()
                .find(|r| r.kind == Reception && r.recipient.iter().any(|addr| addr == recipient))
                .context("sink reception")?,
        );
    }
    anyhow::ensure!(
        deliveries.iter().all(|r| r.num_attempts == 0),
        "{deliveries:?}"
    );
    // A and B have the same destinations but different preference orderings.
    anyhow::ensure!(deliveries[0].site == deliveries[1].site, "{deliveries:?}");
    anyhow::ensure!(
        receptions[0].session_id.is_some() && receptions[0].session_id == receptions[1].session_id,
        "{receptions:?}"
    );
    // C shares labels with A but has a different destination set, so it needs
    // its own queue and configuration.
    anyhow::ensure!(deliveries[0].site != deliveries[2].site, "{deliveries:?}");
    anyhow::ensure!(
        receptions[0].session_id != receptions[2].session_id,
        "{receptions:?}"
    );
    for (record, expected) in
        receptions
            .iter()
            .zip(["route-a.example", "route-a.example", "route-c.example"])
    {
        anyhow::ensure!(
            record
                .meta
                .get("ehlo_domain")
                .and_then(|value| value.as_str())
                == Some(expected),
            "wrong egress configuration: {record:?}"
        );
    }
    Ok(())
}
