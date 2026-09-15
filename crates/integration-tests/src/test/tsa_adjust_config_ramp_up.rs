use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

// Exercises the real up-path end-to-end through tsa-daemon's actual
// background prune tick (state_pruner, a hardcoded 60s loop) rather than
// calling prune_adaptive_overrides directly as the unit tests do. Uses the
// shortest valid ramp_up_interval ("1s") so the tick's own 60s cadence is
// the only real wait involved: decrease_percent=50, floor_percent=25,
// ramp_step_percent defaults to decrease_percent (50), so from a base
// connection_limit of 32: one trigger -> 16, then successive prune ticks
// step 16 -> 24 -> (36, clamped to 32, the original value). The override
// entry itself is *not* removed just because it fully recovered -- only
// the rule's `duration` deadline ever removes an entry -- so
// connection_limit stays present in the override export at 32 rather than
// disappearing back to the base default.
//
// This test takes a few minutes (waits across multiple real 60s ticks) --
// it is the only test proving the actual background timer loop drives a
// ramp to completion, not just the pure step function in isolation.
#[tokio::test]
async fn tsa_adjust_config_ramps_up_after_quiet_period() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("450-adjust-rampup@delivery.mx-sink.wezfurlong.org"),
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

    // Confirm the down-step landed before waiting on the ramp-up.
    let mut initial = None;
    for _ in 0..10 {
        initial = connection_limit(&daemon).await?;
        if initial.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert_eq!(initial, Some(16));

    // Wait for the real background prune tick to step the value up at
    // least once (16 -> 24). The tick runs every 60s; poll generously.
    let mut stepped = false;
    for _ in 0..90 {
        if connection_limit(&daemon).await? == Some(24) {
            stepped = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert!(stepped, "connection_limit never stepped up to 24");

    // Wait for a second tick to fully recover (24 -> would-be 36, clamped
    // to 32): the override entry stays in place (only `duration` removes
    // an entry, not full recovery), so connection_limit is still present
    // in the export, now equal to the original value.
    let mut recovered = false;
    for _ in 0..90 {
        if connection_limit(&daemon).await? == Some(32) {
            recovered = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert!(
        recovered,
        "connection_limit override never clamped to the original value (32) on full recovery"
    );

    daemon.stop().await?;
    Ok(())
}
