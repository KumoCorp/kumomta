use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

// AdjustConfig's absolute-count mode (decrease_amount/floor_amount),
// exercised end-to-end alongside the percentage mode covered by the other
// tsa_adjust_config_* tests. base connection_limit default is 32;
// decrease_amount=10 -> 22 on a single trigger.
#[tokio::test]
async fn tsa_adjust_config_amount_mode() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("450-adjust-amount@delivery.mx-sink.wezfurlong.org"),
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

    let mut connection_limit = None;
    for _ in 0..10 {
        let shaping = daemon.tsa.get_shaping().await?;
        let partial = shaping
            .get_egress_path_config_value(
                "delivery.mx-sink.wezfurlong.org",
                "unspecified",
                "delivery.mx-sink.wezfurlong.org",
            )
            .await?;
        if let Some(value) = partial.get("connection_limit") {
            connection_limit = Some(value.clone());
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let connection_limit = connection_limit.expect("connection_limit should have been adjusted");
    assert_eq!(connection_limit, 22);

    daemon.stop().await?;
    Ok(())
}
