use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

// AdjustConfig's compounding decrease and floor clamp, exercised end-to-end
// via real repeated triggers (not just direct calls into the state
// machine): decrease_percent=50, floor_percent=25 against a base
// connection_limit of 32 means the floor (8) is reached in exactly two
// triggers (32 -> 16 -> 8), and a third trigger must not go any lower.
#[tokio::test]
async fn tsa_adjust_config_compounds_and_clamps_to_floor() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    async fn send_trigger(daemon: &DaemonWithTsa) -> anyhow::Result<()> {
        let mut client = daemon.smtp_client().await?;
        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            recip: Some("450-adjust-compound@delivery.mx-sink.wezfurlong.org"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250);
        Ok(())
    }

    async fn connection_limit(daemon: &DaemonWithTsa) -> anyhow::Result<Option<i64>> {
        let shaping = daemon.tsa.get_shaping().await?;
        let partial = shaping
            .get_egress_path_config_value(
                "delivery.mx-sink.wezfurlong.org",
                "unspecified",
                "delivery.mx-sink.wezfurlong.org",
            )
            .await?;
        Ok(partial.get("connection_limit").and_then(|v| v.as_i64()))
    }

    async fn wait_for_transient_failures(daemon: &DaemonWithTsa, count: usize) {
        daemon
            .with_maildir
            .wait_for_source_summary(
                |summary| summary.get(&TransientFailure).copied().unwrap_or(0) >= count,
                Duration::from_secs(50),
            )
            .await;
    }

    async fn wait_for_connection_limit(
        daemon: &DaemonWithTsa,
        expected: i64,
    ) -> anyhow::Result<()> {
        for _ in 0..20 {
            if connection_limit(daemon).await? == Some(expected) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        anyhow::bail!("connection_limit never reached {expected}");
    }

    // Trigger 1: 32 -> 16
    send_trigger(&daemon).await?;
    wait_for_transient_failures(&daemon, 1).await;
    wait_for_connection_limit(&daemon, 16).await?;

    // Trigger 2: 16 -> 8 (== floor_percent=25% of 32)
    send_trigger(&daemon).await?;
    wait_for_transient_failures(&daemon, 2).await;
    wait_for_connection_limit(&daemon, 8).await?;

    // Trigger 3: already at the floor. Confirm this trigger was actually
    // processed (transient failure count reaches 3) before asserting the
    // value did not drop any lower than the floor.
    send_trigger(&daemon).await?;
    wait_for_transient_failures(&daemon, 3).await;
    // Give the daemon a moment to process the third trigger even though we
    // don't expect the value to change, then confirm it's still exactly 8.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(connection_limit(&daemon).await?, Some(8));

    daemon.stop().await?;
    Ok(())
}
