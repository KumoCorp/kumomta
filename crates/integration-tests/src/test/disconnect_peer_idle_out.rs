use crate::kumod::{DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType::Delivery;
use std::time::Duration;

#[tokio::test]
async fn disconnect_peer_idle_out() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams {
        recip: Some("first@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    // Wait for the sink to idle out its connection.
    // We're assuming that it is set to 3s, so we wait
    // 4s to make things a little less racy
    tokio::time::sleep(Duration::from_secs(4)).await;

    let response = MailGenParams {
        recip: Some("second@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 2,
            Duration::from_secs(50),
        )
        .await;

    daemon
        .wait_for_maildir_count(2, Duration::from_secs(10))
        .await;

    daemon.stop_both().await?;

    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 2,
        Delivery: 2,
    },
    sink_counts: {
        Reception: 2,
        Delivery: 2,
    },
}
"
    );

    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 2,
    delivered: 2,
}
"
    );

    let logs = daemon.source.collect_logs().await?;
    let mut delivered_sessions = logs
        .iter()
        .filter_map(|record| match record.kind {
            kumo_log_types::RecordType::Delivery => Some(record.session_id),
            _ => None,
        })
        .collect::<Vec<_>>();

    delivered_sessions.sort();
    delivered_sessions.dedup();
    assert_eq!(delivered_sessions.len(), 2, "two separate sessions");

    Ok(())
}
