//! End-to-end coverage of the log consumer backlog monitor.

use camino::Utf8PathBuf;
use futures::StreamExt;
use kumo_jsonl::{ConsumerConfig, LogWriterConfig, MultiConsumerTailerConfig, WriterLocation};
use kumo_server_common::log_backlog::{register_location, set_scan_interval, start_monitor};
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;

fn utf8(tmp: &TempDir) -> Utf8PathBuf {
    Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap()
}

// Every test in this file uses the same 200ms interval in place of the 30s
// production default. A test then waits at most a couple hundred
// milliseconds for a gauge update, rather than up to 30 seconds.
fn use_fast_scan() {
    set_scan_interval(Duration::from_millis(200));
}

/// Produce `count` records in `dir` as several done segments. Returns the
/// writer. The caller controls how long `dir` stays registered by choosing
/// when to drop the returned writer.
fn produce_segments(dir: &Utf8PathBuf, count: usize) -> kumo_jsonl::LogWriter {
    let mut writer = LogWriterConfig::new(dir.clone()).max_file_size(1).build();
    for n in 0..count {
        writer.write_value(&json!({ "n": n })).unwrap();
    }
    // Mark the final segment done without dropping the writer to keep every
    // segment readable while the directory stays registered.
    writer.close().unwrap();
    writer
}

/// Commit only the first record read from `dir` under `checkpoint_name`. Later
/// segments are left unread, giving a caller a consumer with a known, non-zero
/// backlog to assert on.
async fn commit_trailing_checkpoint(dir: &Utf8PathBuf, checkpoint_name: &str) {
    let consumer = ConsumerConfig::new("int-test")
        .checkpoint_name(checkpoint_name)
        .max_batch_size(1)
        .max_batch_latency(Duration::from_millis(50));
    let tailer = MultiConsumerTailerConfig::new(dir.clone(), vec![consumer])
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);
    let batches = tokio::select! {
        b = tailer.next() => b.expect("tailer ended before yielding a batch").unwrap(),
        _ = &mut timeout => panic!("timed out waiting for a batch"),
    };
    for mut batch in batches {
        batch.commit().unwrap();
    }
}

/// Read the value of `name{log_dir,checkpoint_name}` from the process metric
/// registry, or `None` when the series has not been published.
fn gauge_value(name: &str, log_dir: &str, checkpoint_name: &str) -> Option<i64> {
    use kumo_prometheus::prometheus::{Encoder, TextEncoder};

    let mut buf = Vec::new();
    TextEncoder::new()
        .encode(
            &kumo_prometheus::prometheus::default_registry().gather(),
            &mut buf,
        )
        .ok()?;
    let text = String::from_utf8(buf).ok()?;

    for line in text.lines() {
        if !line.starts_with(name) {
            continue;
        }
        if line.contains(&format!("log_dir=\"{log_dir}\""))
            && line.contains(&format!("checkpoint_name=\"{checkpoint_name}\""))
        {
            return line
                .rsplit(' ')
                .next()?
                .parse::<f64>()
                .ok()
                .map(|v| v as i64);
        }
    }
    None
}

/// Poll `gauge_value` until it satisfies `condition` or the deadline passes,
/// returning the last observed value.
async fn wait_gauge(
    name: &str,
    log_dir: &str,
    checkpoint_name: &str,
    condition: impl Fn(Option<i64>) -> bool,
) -> Option<i64> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let value = gauge_value(name, log_dir, checkpoint_name);
        if condition(value) {
            return value;
        }
        if tokio::time::Instant::now() >= deadline {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Verifies that `register_location` makes the monitor publish the backlog of a
/// consumer reading the registered directory.
#[tokio::test]
async fn explicit_location_publishes_backlog() {
    use_fast_scan();
    let tmp = TempDir::new().unwrap();
    let dir = utf8(&tmp);

    let writer = produce_segments(&dir, 8);
    commit_trailing_checkpoint(&dir, "configure-local-logs").await;
    // Remove automatic registration to ensure this test exercises
    // `register_location`.
    drop(writer);

    register_location(WriterLocation {
        directory: dir.clone(),
        pattern: "*".to_string(),
    });

    let value = wait_gauge(
        "log_consumer_segments_behind",
        dir.as_str(),
        "configure-local-logs",
        |v| v.is_some_and(|v| v >= 1),
    )
    .await;
    k9::assert_equal!(value.is_some_and(|v| v >= 1), true);
}

/// Verifies that the monitor publishes backlog for a live writer without an
/// explicit call to `register_location`.
#[tokio::test]
async fn registered_writer_publishes_backlog() {
    use_fast_scan();
    let tmp = TempDir::new().unwrap();
    let dir = utf8(&tmp);

    // Keep the registration active while the monitor scans.
    let _writer = produce_segments(&dir, 8);
    commit_trailing_checkpoint(&dir, "new-writer").await;

    start_monitor();

    let value = wait_gauge(
        "log_consumer_segments_behind",
        dir.as_str(),
        "new-writer",
        |v| v.is_some_and(|v| v >= 1),
    )
    .await;
    k9::assert_equal!(value.is_some_and(|v| v >= 1), true);
}

/// Verifies that dropping a writer clears its backlog gauges instead of
/// retaining stale values.
#[tokio::test]
async fn dropped_writer_clears_gauges() {
    use_fast_scan();
    let tmp = TempDir::new().unwrap();
    let dir = utf8(&tmp);

    let writer = produce_segments(&dir, 8);
    commit_trailing_checkpoint(&dir, "transient").await;
    start_monitor();

    let value = wait_gauge(
        "log_consumer_segments_behind",
        dir.as_str(),
        "transient",
        |v| v.is_some(),
    )
    .await;
    k9::assert_equal!(value.is_some(), true);

    drop(writer);
    let value = wait_gauge(
        "log_consumer_segments_behind",
        dir.as_str(),
        "transient",
        |v| v.is_none(),
    )
    .await;
    k9::assert_equal!(value, None);
}
