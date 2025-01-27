use crate::kumod::{DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

/// This test verifies that a 421 disconnect doesn't cause collateral
/// damage for the next message in the ready queue by making it
/// experience a TransientFailure due to a connection error,
/// when reconnect_strategy=ReconnectSameHost
#[tokio::test]
async fn disconnect_reconnect_same_host() -> anyhow::Result<()> {
    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_RECONNECT_STRATEGY", "ReconnectSameHost")])
            .await?;
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

    let response = MailGenParams {
        recip: Some("second@example.com"),
        sender: Some("second@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    let response = MailGenParams {
        recip: Some("third@example.com"),
        sender: Some("third@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&TransientFailure).copied().unwrap_or(0) > 0
                    && summary.get(&Delivery).copied().unwrap_or(0) >= 2
            },
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
        Reception: 3,
        Delivery: 2,
        TransientFailure: 1,
    },
    sink_counts: {
        Reception: 2,
        Delivery: 2,
        Rejection: 1,
    },
}
"
    );

    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 3,
    delivered: 2,
}
"
    );

    Ok(())
}
