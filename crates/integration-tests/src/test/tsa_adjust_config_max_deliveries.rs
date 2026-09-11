use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

// AdjustConfig on max_deliveries_per_connection: no explicit base value for
// this domain, so it falls back to EgressPathConfig's default (1024).
// decrease_amount=24 -> 1024 - 24 = 1000.
#[tokio::test]
async fn tsa_adjust_config_max_deliveries_per_connection() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("450-adjust-deliveries@delivery.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .with_maildir
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    let mut max_deliveries = None;
    for _ in 0..10 {
        let shaping = daemon.tsa.get_shaping().await?;
        let partial = shaping
            .get_egress_path_config_value(
                "delivery.mx-sink.wezfurlong.org",
                "unspecified",
                "delivery.mx-sink.wezfurlong.org",
            )
            .await?;
        if let Some(value) = partial.get("max_deliveries_per_connection") {
            max_deliveries = Some(value.clone());
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let max_deliveries =
        max_deliveries.expect("max_deliveries_per_connection should have been adjusted");
    assert_eq!(max_deliveries, 1000);

    daemon.stop().await?;
    Ok(())
}
