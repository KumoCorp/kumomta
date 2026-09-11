use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

// AdjustConfig on a rate field (max_message_rate, a ThrottleSpec, unlike
// connection_limit's plain LimitSpec) end-to-end: confirms the percentage
// math and TOML string round-trip ("1000/s" -> "800/s") work through the
// real HTTP export, not just the plain-integer connection_limit case
// covered by tsa_adjust_config_automation.
#[tokio::test]
async fn tsa_adjust_config_rate_field() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("450-adjust-msgrate@delivery.mx-sink.wezfurlong.org"),
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

    let mut max_message_rate = None;
    for _ in 0..10 {
        let shaping = daemon.tsa.get_shaping().await?;
        let partial = shaping
            .get_egress_path_config_value(
                "delivery.mx-sink.wezfurlong.org",
                "unspecified",
                "delivery.mx-sink.wezfurlong.org",
            )
            .await?;
        if let Some(value) = partial.get("max_message_rate") {
            max_message_rate = Some(value.clone());
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let max_message_rate = max_message_rate.expect("max_message_rate should have been adjusted");
    // base 1000/s, decrease_percent=20 -> 800/s
    assert_eq!(max_message_rate, "800/s");

    daemon.stop().await?;
    Ok(())
}
