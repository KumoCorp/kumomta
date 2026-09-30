use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use serde_json::json;
use std::time::Duration;

/// A failed named entry must not discard the other entries in an explicit
/// mx_list, regardless of order. With no usable entries, defer rather than
/// repeatedly failing dispatcher initialization with the message still ready.
#[tokio::test]
async fn mx_list_dns_failure() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-mxlist.lua")
        .env("KUMOD_MX_LIST_DNS_FAILURE", "1")
        .start()
        .await?;
    let port = daemon.sink.listener("smtp").port();
    let unavailable = format!("unavailable.example.test:{port}");
    let healthy = format!("healthy.example.test:{port}");
    let empty = format!("empty.example.test:{port}");
    std::fs::write(
        daemon.source.dir.path().join("queue-data.json"),
        serde_json::to_vec(&json!({
            "first.example.test": [&unavailable, &healthy],
            "last.example.test": [&healthy, &unavailable],
            "all.example.test": [&unavailable, &empty],
        }))?,
    )?;
    let mut client = daemon.smtp_client().await?;
    for recipient in [
        "recip@first.example.test",
        "recip@last.example.test",
        "recip@all.example.test",
    ] {
        let response = MailGenParams {
            recip: Some(recipient),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250);
    }
    anyhow::ensure!(
        daemon
            .wait_for_maildir_count(2, Duration::from_secs(10))
            .await,
        "a DNS failure blocked a healthy mx_list alternative"
    );
    anyhow::ensure!(
        daemon
            .wait_for_source_summary(
                |s| s.get(&Delivery).copied().unwrap_or(0) == 2
                    && s.get(&TransientFailure).copied().unwrap_or(0) == 1,
                Duration::from_secs(10)
            )
            .await,
        "an unresolved mx_list did not defer"
    );
    daemon.stop_both().await?;
    let records = daemon.source.collect_logs().await?;
    for recipient in ["recip@first.example.test", "recip@last.example.test"] {
        let delivery = records
            .iter()
            .find(|r| r.kind == Delivery && r.recipient.iter().any(|addr| addr == recipient))
            .context("delivery through healthy alternative")?;
        anyhow::ensure!(
            delivery.num_attempts == 0
                && delivery
                    .peer_address
                    .as_ref()
                    .is_some_and(|a| a.name == "healthy.example.test"),
            "{delivery:?}"
        );
    }
    let failure = records
        .iter()
        .find(|r| {
            r.kind == TransientFailure
                && r.recipient
                    .iter()
                    .any(|addr| addr == "recip@all.example.test")
        })
        .context("all entries failed")?;
    anyhow::ensure!(
        failure.response.code == 451
            && failure
                .response
                .content
                .contains("unavailable.example.test")
            && failure.response.content.contains("Server Failure"),
        "missing DNS failure diagnostic: {failure:?}"
    );
    Ok(())
}
