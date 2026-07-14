use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::Delayed;
use std::collections::BTreeSet;
use std::time::Duration;

/// Verify that a dispatcher whose `send` function blocks forever is
/// caught by the progress watchdog: the task is aborted, the in-flight
/// message is requeued via `Dispatcher::drop`, and the SMTP-style
/// response recorded for that requeue identifies the watchdog cause
/// and carries the dispatcher's session_id for correlation with the
/// ERROR diagnostic that the watchdog emits at abort time.
#[tokio::test]
async fn dispatcher_watchdog_aborts_wedged_lua_send() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-watchdog.lua")
        .start()
        .await
        .context("DaemonWithMaildirOptions::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams {
        recip: Some("victim@wedge.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250, "unexpected response: {response:?}");

    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(30),
            |m| {
                m.name().as_str() == "dispatcher_watchdog_aborted_total"
                    && m.labels()
                        .get("service")
                        .map(|s| s.contains("make.wedge_send"))
                        .unwrap_or(false)
            },
            |values| values.iter().sum::<f64>() >= 1.0,
        )
        .await
        .context("waiting for watchdog abort to be recorded")?;

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delayed).copied().unwrap_or(0) >= 1,
            Duration::from_secs(10),
        )
        .await;

    daemon.stop_both().await.context("stop_both")?;

    // The wedged dispatcher will have respawned and been aborted
    // multiple times by the time we stop; dedupe by (kind, normalized
    // response line). The shared response normalizer collapses
    // durations, timestamps, and UUIDs so the snapshot is stable.
    let dispositions: BTreeSet<(String, String)> = daemon
        .source
        .collect_logs()
        .await?
        .into_iter()
        .map(|r| {
            (
                format!("{:?}", r.kind),
                mod_smtp_response_normalize::normalize(&r.response.to_single_line()),
            )
        })
        .collect();

    k9::snapshot!(
        dispositions,
        r#"
{
    (
        "Delayed",
        "451 4.4.1 dispatcher watchdog aborted task; phase=DeliveringMessage detail="lua: send" session={uuid} age={duration} time_in_phase={duration}",
    ),
    (
        "Reception",
        "250",
    ),
}
"#
    );

    Ok(())
}
