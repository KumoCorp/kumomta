use crate::kumod::{DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use camino::Utf8PathBuf;
use futures::StreamExt;
use kumo_jsonl::{ConsumerConfig, MultiConsumerTailerConfig};
use kumo_log_types::RecordType::Delivery;
use std::time::Duration;

/// Verifies that a daemon configured with `configure_local_logs` publishes the
/// consumer backlog gauge for a checkpoint left in its log directory.
#[tokio::test]
async fn configure_local_logs_backlog_metric() -> anyhow::Result<()> {
    // Scan often to avoid the 30s production cadence.
    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_LOG_BACKLOG_SCAN_INTERVAL", "1")]).await?;
    let mut client = daemon.smtp_client().await?;

    for _ in 0..3 {
        let response = MailGenParams::default().send(&mut client).await?;
        anyhow::ensure!(response.code == 250, "unexpected response: {response:#?}");
    }

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 3,
            Duration::from_secs(50),
        )
        .await;

    let log_dir = Utf8PathBuf::try_from(daemon.source.dir.path().join("logs"))
        .context("log dir is not valid UTF-8")?;

    // Use a real tailer to create the checkpoint that would be created by an
    // independent consumer process.
    let consumer = ConsumerConfig::new("int-test")
        .checkpoint_name("int-consumer")
        .max_batch_size(1)
        .max_batch_latency(Duration::from_millis(500));
    let tailer = MultiConsumerTailerConfig::new(log_dir, vec![consumer])
        .build()
        .await?;
    tokio::pin!(tailer);

    let timeout = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(timeout);
    let batches = tokio::select! {
        b = tailer.next() => b.context("tailer ended before yielding a batch")??,
        _ = &mut timeout => anyhow::bail!("timed out waiting for a batch from the log dir"),
    };
    for mut batch in batches {
        batch.commit()?;
    }

    // It asserts the series is published rather than a specific value,
    // since how many segments the live daemon has rolled depends on timing.
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(30),
            |m| {
                m.name().as_str() == "log_consumer_segments_behind"
                    && m.label_is("checkpoint_name", "int-consumer")
            },
            |values| !values.is_empty(),
        )
        .await
        .context(
            "log_consumer_segments_behind{checkpoint_name=int-consumer} should be published",
        )?;

    daemon.stop_both().await?;
    Ok(())
}
