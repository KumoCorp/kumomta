use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use std::time::Duration;

/// Verifies that a running kumod exports the per-thread-pool runtime
/// metrics and the lruttl populate-duration histogram on its `/metrics`
/// endpoint after handling a message.
#[tokio::test]
async fn runtime_and_populate_metrics() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250);
    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    // The per-pool runtime gauges are refreshed by a sampler once per second.
    // Waiting for the smtpsrv pool's busy-time series confirms that a named
    // runtime, not only the main runtime, is being sampled.
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| m.name().as_str() == "thread_pool_busy_seconds" && m.label_is("pool", "smtpsrv"),
            |values| !values.is_empty(),
        )
        .await
        .context("waiting for thread_pool_busy_seconds{pool=smtpsrv}")?;

    // The populate-duration histogram recorded the egress pool populate.
    let counts: Vec<f64> = daemon
        .source
        .api_client()
        .get_metrics(|m| {
            if m.name().as_str() == "lruttl_populate_duration"
                && m.label_is("cache_name", "egress_source_pools")
            {
                Some(m.as_histogram().count)
            } else {
                None
            }
        })
        .await
        .context("scraping lruttl_populate_duration")?;
    anyhow::ensure!(
        counts.iter().any(|count| *count >= 1.0),
        "expected at least one egress_source_pools populate observation, got {counts:?}"
    );

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}
