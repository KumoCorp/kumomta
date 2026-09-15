use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

// End-to-end coverage for the AdjustConfig down-path: a real kumod + tsa-daemon
// pair, a message rejected with a response matching the AdjustConfig rule in
// shaping.toml, and confirmation that the resulting connection_limit
// reduction is visible via TSA's exported shaping.toml.
//
// connection_limit has no explicit base value for this domain in
// shaping.toml, so the daemon falls back to EgressPathConfig's struct-level
// default (32). decrease_percent=50 -> 16, well above the floor_percent=25
// (8) clamp, so no floor clamping is exercised here (that's covered by the
// unit tests in crates/tsa-daemon/src/state.rs).
#[tokio::test]
async fn tsa_adjust_config_automation() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("450-adjust-connlimit@delivery.mx-sink.wezfurlong.org"),
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
    assert_eq!(connection_limit, 16);

    daemon.stop().await?;
    Ok(())
}
