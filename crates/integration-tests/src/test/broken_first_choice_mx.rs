use crate::kumod::{DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType::Delivery;
use std::time::Duration;

/// We're sending to broken-first-mx which is defined in source.lua
/// to have a non-routable first MX in its connection plan, followed
/// by the regular sink.  This should cause a connection failure
/// for the first candidate, but we should then successfully
/// deliver to the second candidate.
#[tokio::test]
async fn broken_first_choice_mx() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams {
        recip: Some("first@broken-first-mx.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 1,
            Duration::from_secs(50),
        )
        .await;

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await?;

    let delivery_summary = daemon.dump_logs().await?;
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

    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 1,
    delivered: 1,
}
"
    );

    let logs = daemon.source.collect_logs().await?;
    let delivered = logs.iter().find(|record| record.kind == Delivery).unwrap();

    k9::assert_equal!(delivered.peer_address.as_ref().unwrap().name, "workinghost");

    Ok(())
}
