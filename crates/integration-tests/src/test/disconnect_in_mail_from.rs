use crate::kumod::{DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

#[tokio::test]
async fn disconnect_in_mail_from() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams {
        recip: Some("pick-me@example.com"),
        // Cause the sink to 421 disconnect us in mail from.
        // This is to verify that we handle this sort of error during
        // a pipeline send correctly.
        sender: Some("421-disconnect-me@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(5),
        )
        .await;

    daemon.stop_both().await?;
    let delivery_summary = daemon.dump_logs()?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        TransientFailure: 1,
    },
    sink_counts: {
        Rejection: 1,
    },
}
"
    );

    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 1,
    delivered: 0,
}
"
    );
    Ok(())
}
