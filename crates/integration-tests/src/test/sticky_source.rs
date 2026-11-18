use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::JsonLogRecord;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// Verifies that a sticky egress pool routes a message's retries,
/// including the eventual delivery, through the same source that made
/// its first attempt.
#[tokio::test]
async fn sticky_source_retries_via_same_source() -> anyhow::Result<()> {
    const MESSAGE_COUNT: usize = 6;

    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-sticky-pool.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    // Each pool source has a distinct EHLO domain and the sink greylists
    // per (EHLO domain, recipient key). A retry from a different source
    // counts as a first attempt for that domain and is greylisted again.
    for n in 0..MESSAGE_COUNT {
        let recip = format!("greylist-user{n}-1s@example.com");
        let response = MailGenParams {
            recip: Some(&recip),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250);
    }

    daemon
        .wait_for_maildir_count(MESSAGE_COUNT, Duration::from_secs(60))
        .await;

    let delivered = daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= MESSAGE_COUNT,
            Duration::from_secs(30),
        )
        .await;
    anyhow::ensure!(
        delivered,
        "expected {MESSAGE_COUNT} Delivery records in the source log"
    );

    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;

    let mut by_message: BTreeMap<&str, Vec<&JsonLogRecord>> = BTreeMap::new();
    for record in &logs {
        if record.kind == Delivery || record.kind == TransientFailure {
            by_message.entry(&record.id).or_default().push(record);
        }
    }
    k9::assert_equal!(by_message.len(), MESSAGE_COUNT);

    let mut sources_used = BTreeSet::new();
    for (id, records) in by_message {
        // A retry can also get a 421 from the sink's idle connection
        // timeout, which is not the greylist response.
        let greylisted = records
            .iter()
            .filter(|r| r.kind == TransientFailure && r.response.code == 451)
            .count();
        let deliveries: Vec<_> = records.iter().filter(|r| r.kind == Delivery).collect();

        anyhow::ensure!(
            greylisted == 1,
            "{id}: expected exactly one greylist deferral, got {records:#?}"
        );
        anyhow::ensure!(
            deliveries.len() == 1,
            "{id}: expected exactly one delivery, got {deliveries:#?}"
        );

        let delivery = deliveries[0];
        let source = delivery
            .egress_source
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("{id}: delivery has no egress_source"))?;

        for record in &records {
            k9::assert_equal!(record.egress_source.as_deref(), Some(source));
            k9::assert_equal!(
                record.meta.get("sticky_egress_source"),
                Some(&serde_json::Value::String(source.to_string()))
            );
        }
        sources_used.insert(source.to_string());
    }

    // Confirm both sources got picked at least once; otherwise this run
    // would only exercise stickiness for one of them.
    k9::assert_equal!(
        sources_used,
        BTreeSet::from(["a".to_string(), "b".to_string()])
    );

    Ok(())
}
