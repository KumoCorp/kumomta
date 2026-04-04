use camino::Utf8PathBuf;
use futures::StreamExt;
use kumo_jsonl::{
    ConsumerConfig, LogBatch, LogTailer, LogTailerConfig, LogWriter, LogWriterConfig,
    MultiConsumerTailerConfig,
};
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;

/// Create a [`LogWriter`] for the given directory.
fn writer_for(dir: &std::path::Path) -> LogWriter {
    let log_dir = Utf8PathBuf::try_from(dir.to_path_buf()).unwrap();
    LogWriterConfig::new(log_dir)
        .compression_level(3)
        .max_file_size(u64::MAX)
        .build()
}

/// Write records into a single segment and close it (marks it done).
fn write_segment(dir: &std::path::Path, records: &[&str]) {
    let mut w = writer_for(dir);
    for r in records {
        w.write_line(r).unwrap();
    }
    w.close().unwrap();
}

/// Write records into a single segment but leave it writable
/// (not done), simulating an in-progress file.
fn write_open_segment(dir: &std::path::Path, records: &[&str]) {
    let mut w = writer_for(dir);
    for r in records {
        w.write_line(r).unwrap();
    }
    w.flush_without_marking_done().unwrap();
}

fn utf8_dir(dir: &TempDir) -> Utf8PathBuf {
    Utf8PathBuf::try_from(dir.path().to_path_buf()).unwrap()
}

/// Helper to collect exactly one batch from a tailer with a timeout.
async fn next_batch_with_timeout(tailer: &mut std::pin::Pin<&mut LogTailer>) -> LogBatch {
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);
    tokio::select! {
        batch = tailer.next() => {
            batch.expect("expected a batch").expect("batch should be Ok")
        }
        _ = &mut timeout => {
            panic!("timed out waiting for a batch");
        }
    }
}

// -----------------------------------------------------------------------

/// Read one record at a time, closing and reopening with checkpoint.
/// Verify we get all records exactly once, in order.
#[tokio::test]
async fn test_checkpoint_resume_one_at_a_time() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    let records = vec![
        r#"{"id":1}"#,
        r#"{"id":2}"#,
        r#"{"id":3}"#,
        r#"{"id":4}"#,
        r#"{"id":5}"#,
    ];

    write_segment(dir.path(), &records);

    let mut all_records = Vec::new();

    for i in 0..5 {
        let tailer = LogTailerConfig::new(log_dir.clone())
            .max_batch_size(1)
            .max_batch_latency(Duration::from_millis(50))
            .checkpoint_name("test-cp")
            .build()
            .await
            .unwrap();
        tokio::pin!(tailer);

        let mut batch = tailer
            .next()
            .await
            .unwrap_or_else(|| panic!("expected a batch on iteration {i}"))
            .unwrap_or_else(|e| panic!("expected Ok batch on iteration {i}: {e}"));
        k9::assert_equal!(batch.len(), 1);
        all_records.push(batch.records()[0].clone());
        batch.commit().unwrap();

        tailer.as_mut().close();
    }

    let expected: Vec<serde_json::Value> = records
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    k9::assert_equal!(all_records, expected);

    // One more tailer should yield no records from the completed file.
    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(1)
        .max_batch_latency(Duration::from_millis(50))
        .checkpoint_name("test-cp")
        .build()
        .await
        .unwrap();
    tailer.close();
    tokio::pin!(tailer);
    let result = tailer.next().await;
    k9::assert_equal!(result.is_none(), true);
}

/// Verify that records from multiple completed segment files are
/// read in file-sorted (i.e. chronological) order.
#[tokio::test]
async fn test_multiple_files_in_order() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Use a single writer with close() between segments to ensure ordering.
    let mut w = writer_for(dir.path());
    w.write_line(r#"{"file":"a","n":1}"#).unwrap();
    w.close().unwrap();
    let mut w = writer_for(dir.path());
    w.write_line(r#"{"file":"b","n":1}"#).unwrap();
    w.write_line(r#"{"file":"b","n":2}"#).unwrap();
    w.close().unwrap();
    let mut w = writer_for(dir.path());
    w.write_line(r#"{"file":"c","n":1}"#).unwrap();
    w.close().unwrap();

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    // Collect all records from completed files.
    // We expect to get batches covering all 4 records from 3 files.
    // The tailer may yield them in one or more batches.
    let mut all_records = Vec::new();
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            batch = tailer.next() => {
                match batch {
                    Some(Ok(records)) => {
                        all_records.extend(records.records().iter().cloned());
                        if all_records.len() >= 4 {
                            break;
                        }
                    }
                    Some(Err(e)) => panic!("unexpected error: {e}"),
                    None => break,
                }
            }
            _ = &mut timeout => {
                panic!("timed out waiting for records; got {} so far", all_records.len());
            }
        }
    }

    k9::assert_equal!(
        all_records,
        vec![
            json!({"file": "a", "n": 1}),
            json!({"file": "b", "n": 1}),
            json!({"file": "b", "n": 2}),
            json!({"file": "c", "n": 1}),
        ]
    );
}

/// Checkpoint resume spanning two separate log files.
#[tokio::test]
async fn test_checkpoint_across_multiple_files() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    let mut w = writer_for(dir.path());
    w.write_line(r#"{"id":1}"#).unwrap();
    w.write_line(r#"{"id":2}"#).unwrap();
    w.close().unwrap();
    let mut w = writer_for(dir.path());
    w.write_line(r#"{"id":3}"#).unwrap();
    w.write_line(r#"{"id":4}"#).unwrap();
    w.close().unwrap();

    let mut all_records = Vec::new();

    for i in 0..4 {
        let tailer = LogTailerConfig::new(log_dir.clone())
            .max_batch_size(1)
            .max_batch_latency(Duration::from_millis(50))
            .checkpoint_name("multi-cp")
            .build()
            .await
            .unwrap();
        tokio::pin!(tailer);

        let mut batch = tailer
            .next()
            .await
            .unwrap_or_else(|| panic!("expected batch on iteration {i}"))
            .unwrap_or_else(|e| panic!("error on iteration {i}: {e}"));
        k9::assert_equal!(batch.len(), 1);
        all_records.push(batch.records()[0].clone());
        batch.commit().unwrap();
        tailer.as_mut().close();
    }

    k9::assert_equal!(
        all_records,
        vec![
            json!({"id":1}),
            json!({"id":2}),
            json!({"id":3}),
            json!({"id":4})
        ]
    );
}

/// Verify that commit() after reading one record advances the checkpoint
/// so the next tailer sees the *next* record, not the same one.
#[tokio::test]
async fn test_commit_advances_checkpoint_past_consumed_batch() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_segment(dir.path(), &[r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#]);

    // First tailer: read one record, then close
    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(1)
        .max_batch_latency(Duration::from_millis(50))
        .checkpoint_name("advance-cp")
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let mut first = tailer
        .next()
        .await
        .expect("should yield a batch")
        .expect("batch should be Ok");
    k9::assert_equal!(first.records(), &[json!({"n": 1})]);
    first.commit().unwrap();
    tailer.as_mut().close();

    let tailer2 = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(1)
        .max_batch_latency(Duration::from_millis(50))
        .checkpoint_name("advance-cp")
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer2);

    let mut second = tailer2
        .next()
        .await
        .expect("should yield a batch")
        .expect("batch should be Ok");
    k9::assert_equal!(second.records(), &[json!({"n": 2})]);
    second.commit().unwrap();
    tailer2.as_mut().close();
}

/// Verify that dropping a batch *without* calling commit() does NOT
/// advance the checkpoint. Reopening with the same checkpoint should
/// re-read the same record.
#[tokio::test]
async fn test_drop_without_commit_does_not_advance_checkpoint() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_segment(dir.path(), &[r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#]);

    // First tailer: read one record, then drop without close
    {
        let tailer = LogTailerConfig::new(log_dir.clone())
            .max_batch_size(1)
            .max_batch_latency(Duration::from_millis(50))
            .checkpoint_name("drop-cp")
            .build()
            .await
            .unwrap();
        tokio::pin!(tailer);

        let first = tailer
            .next()
            .await
            .expect("should yield a batch")
            .expect("batch should be Ok");
        k9::assert_equal!(first.records(), &[json!({"n": 1})]);
        // batch is dropped here without calling commit()
    }

    let tailer2 = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(1)
        .max_batch_latency(Duration::from_millis(50))
        .checkpoint_name("drop-cp")
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer2);

    let second = tailer2
        .next()
        .await
        .expect("should yield a batch")
        .expect("batch should be Ok");
    k9::assert_equal!(second.records(), &[json!({"n": 1})]);
    tailer2.as_mut().close();
}

/// Verify that `tail(true)` skips older segments and starts reading
/// from the most recent one.  Also verify that tail mode does not
/// create a checkpoint file.
#[tokio::test]
async fn test_tail_starts_from_latest_segment() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Two completed segments; the tailer should skip the first.
    let mut w = writer_for(dir.path());
    w.write_line(r#"{"seg":1,"n":1}"#).unwrap();
    w.write_line(r#"{"seg":1,"n":2}"#).unwrap();
    w.close().unwrap();
    let mut w = writer_for(dir.path());
    w.write_line(r#"{"seg":2,"n":1}"#).unwrap();
    w.write_line(r#"{"seg":2,"n":2}"#).unwrap();
    w.close().unwrap();

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .tail(true)
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let mut all_records = Vec::new();
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            batch = tailer.next() => {
                match batch {
                    Some(Ok(records)) => {
                        all_records.extend(records.records().iter().cloned());
                        if all_records.len() >= 2 {
                            break;
                        }
                    }
                    Some(Err(e)) => panic!("unexpected error: {e}"),
                    None => break,
                }
            }
            _ = &mut timeout => {
                panic!("timed out waiting for records; got {} so far", all_records.len());
            }
        }
    }

    // Should only contain records from the second segment, not the first
    k9::assert_equal!(
        all_records,
        vec![json!({"seg": 2, "n": 1}), json!({"seg": 2, "n": 2})]
    );

    // Tail mode must NOT have created any checkpoint file.
    // Directory should contain only the two log segments.
    let dir_entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    k9::assert_equal!(dir_entries.len(), 2);
}

/// Verify that a single batch can contain records from multiple segment
/// files when the batch size is large enough to span both.
#[tokio::test]
async fn test_batch_spans_multiple_segments() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    let mut w = writer_for(dir.path());
    w.write_line(r#"{"seg":1,"n":1}"#).unwrap();
    w.write_line(r#"{"seg":1,"n":2}"#).unwrap();
    w.close().unwrap();
    let mut w = writer_for(dir.path());
    w.write_line(r#"{"seg":2,"n":1}"#).unwrap();
    w.write_line(r#"{"seg":2,"n":2}"#).unwrap();
    w.close().unwrap();

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(10)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let batch = next_batch_with_timeout(&mut tailer).await;

    // All 4 records should be in a single batch
    k9::assert_equal!(batch.len(), 4);
    k9::assert_equal!(
        batch.records(),
        &[
            json!({"seg": 1, "n": 1}),
            json!({"seg": 1, "n": 2}),
            json!({"seg": 2, "n": 1}),
            json!({"seg": 2, "n": 2}),
        ]
    );

    // The batch should reference two distinct segment files
    k9::assert_equal!(batch.file_names().len(), 2);
}

/// Verify that max_batch_size still constrains the batch even when
/// multiple segments are available. Records beyond the limit should
/// appear in subsequent batches.
#[tokio::test]
async fn test_batch_size_constrains_across_segments() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    let mut w = writer_for(dir.path());
    w.write_line(r#"{"seg":1,"n":1}"#).unwrap();
    w.write_line(r#"{"seg":1,"n":2}"#).unwrap();
    w.close().unwrap();
    let mut w = writer_for(dir.path());
    w.write_line(r#"{"seg":2,"n":1}"#).unwrap();
    w.close().unwrap();

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(2)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    // First batch: exactly 2 records (the limit)
    let batch1 = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch1.len(), 2);
    k9::assert_equal!(
        batch1.records(),
        &[json!({"seg": 1, "n": 1}), json!({"seg": 1, "n": 2})]
    );

    // Second batch: the remaining record from the next segment
    let batch2 = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch2.len(), 1);
    k9::assert_equal!(batch2.records(), &[json!({"seg": 2, "n": 1})]);
}

/// Verify that a partial batch (fewer records than max_batch_size) is
/// yielded after max_batch_latency expires when the file is still
/// being written to (not yet marked readonly).
#[tokio::test]
async fn test_partial_batch_flushed_by_latency() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_open_segment(dir.path(), &[r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#]);

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(200))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let start = tokio::time::Instant::now();
    let batch = next_batch_with_timeout(&mut tailer).await;
    let elapsed = start.elapsed();

    // Should have yielded all 3 records as a partial batch
    k9::assert_equal!(batch.len(), 3);
    k9::assert_equal!(
        batch.records(),
        &[json!({"n": 1}), json!({"n": 2}), json!({"n": 3})]
    );

    // The batch should have been yielded after roughly the latency
    // period, not immediately (it waited for more data).
    assert!(
        elapsed >= Duration::from_millis(150),
        "expected to wait for latency timer, but elapsed was {elapsed:?}"
    );
}

/// Verify that a partial batch from a completed file is yielded
/// immediately without waiting for the latency timer.
#[tokio::test]
async fn test_partial_batch_from_done_file_yields_immediately() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_segment(dir.path(), &[r#"{"n":1}"#, r#"{"n":2}"#]);

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_secs(10))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let start = tokio::time::Instant::now();
    let batch = next_batch_with_timeout(&mut tailer).await;
    let elapsed = start.elapsed();

    // Should have yielded the 2 records without waiting
    k9::assert_equal!(batch.len(), 2);
    k9::assert_equal!(batch.records(), &[json!({"n": 1}), json!({"n": 2})]);

    // Should return quickly, well before the 10s latency timer
    assert!(
        elapsed < Duration::from_secs(1),
        "expected immediate yield for done file, but elapsed was {elapsed:?}"
    );
}

/// Core logic for the late-arriving file test.  Parameterized by
/// `poll_watcher` so it can be run with both the native and poll
/// watcher backends.
async fn late_arriving_file_impl(poll_watcher: Option<Duration>) {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_segment(dir.path(), &[r#"{"n":1}"#, r#"{"n":2}"#]);

    let mut config = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100));
    if let Some(interval) = poll_watcher {
        config = config.poll_watcher(interval);
    }
    let tailer = config.build().await.unwrap();
    tokio::pin!(tailer);

    // Read the first batch — should contain both records from the first segment.
    let batch1 = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch1.records(), &[json!({"n": 1}), json!({"n": 2})]);

    // Now the tailer is waiting for new files.  Give it a moment
    // to enter the wait state, then write a second segment.
    tokio::time::sleep(Duration::from_millis(200)).await;

    write_segment(dir.path(), &[r#"{"n":3}"#, r#"{"n":4}"#]);

    // The tailer should discover the new file and yield its records.
    let batch2 = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch2.records(), &[json!({"n": 3}), json!({"n": 4})]);
}

/// Late-arriving file discovered via the native filesystem watcher.
#[tokio::test]
async fn test_late_arriving_file_native_watcher() {
    late_arriving_file_impl(None).await;
}

/// Late-arriving file discovered via the poll watcher.
#[tokio::test]
async fn test_late_arriving_file_poll_watcher() {
    late_arriving_file_impl(Some(Duration::from_millis(200))).await;
}

/// Verify that calling commit() on a batch from a tailer without
/// a checkpoint configured is a harmless no-op.
#[tokio::test]
async fn test_commit_without_checkpoint_is_noop() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_segment(dir.path(), &[r#"{"n":1}"#]);

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(10)
        .max_batch_latency(Duration::from_millis(50))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let mut batch = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch.records(), &[json!({"n": 1})]);

    // commit() should succeed (no-op) without error
    batch.commit().unwrap();
    // calling it again is also fine
    batch.commit().unwrap();

    // No checkpoint file should have been created — only the segment
    let dir_entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    k9::assert_equal!(dir_entries.len(), 1);
}

/// Test multi-consumer mode with differing max_batch_latency.
///
/// Consumer "fast" has a short latency (200ms) and consumer "slow"
/// has a long latency (10s).  With a writable (not-done) file
/// containing 3 records, the fast consumer should yield its batch
/// after ~200ms while the slow consumer's batch is NOT yet ready.
/// On a subsequent iteration the slow consumer's batch should also
/// be yielded (because the file is then marked done, flushing all).
#[tokio::test]
async fn test_multi_consumer_differing_latency() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_open_segment(dir.path(), &[r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#]);

    let fast = ConsumerConfig::new("fast")
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(200));

    let slow = ConsumerConfig::new("slow")
        .max_batch_size(100)
        .max_batch_latency(Duration::from_secs(10));

    let config = MultiConsumerTailerConfig::new(log_dir.clone(), vec![fast, slow]);

    let tailer = config.build().await.unwrap();
    tokio::pin!(tailer);

    // First yield: only the fast consumer's batch should be ready
    // (its 200ms latency expires), while the slow consumer (10s)
    // is still accumulating.
    let start = tokio::time::Instant::now();
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);

    let batches1 = tokio::select! {
        b = tailer.next() => b.expect("expected batches").expect("should be Ok"),
        _ = &mut timeout => panic!("timed out waiting for first yield"),
    };
    let elapsed = start.elapsed();

    // Should have waited roughly the fast latency, not the slow one
    assert!(
        elapsed >= Duration::from_millis(150),
        "yielded too quickly: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "waited too long: {elapsed:?}"
    );

    k9::assert_equal!(batches1.len(), 1);
    // Only the fast consumer should be in this yield
    k9::assert_equal!(batches1[0].consumer_name(), "fast");
    k9::assert_equal!(
        batches1[0].records(),
        &[json!({"n": 1}), json!({"n": 2}), json!({"n": 3})]
    );

    // Now mark the file as done so the slow consumer's batch
    // gets flushed on the next iteration.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    for entry in entries {
        let mut perms = entry.metadata().unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(true);
        std::fs::set_permissions(entry.path(), perms).unwrap();
    }

    let timeout2 = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout2);

    let batches2 = tokio::select! {
        b = tailer.next() => b.expect("expected batches").expect("should be Ok"),
        _ = &mut timeout2 => panic!("timed out waiting for second yield"),
    };

    // The slow consumer's batch should now be yielded
    k9::assert_equal!(batches2.len(), 1);
    k9::assert_equal!(batches2[0].consumer_name(), "slow");
    k9::assert_equal!(
        batches2[0].records(),
        &[json!({"n": 1}), json!({"n": 2}), json!({"n": 3})]
    );
}

/// Test that LogWriter produces segment files that LogTailer can read.
#[tokio::test]
async fn test_writer_round_trip() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    let mut writer = LogWriterConfig::new(log_dir.clone())
        .compression_level(3)
        .max_file_size(10_000)
        .build();

    // Write records using LogWriter
    writer.write_value(&json!({"id": 1})).unwrap();
    writer.write_value(&json!({"id": 2})).unwrap();
    writer.write_value(&json!({"id": 3})).unwrap();
    writer.close().unwrap();

    // Read them back with LogTailer
    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let batch = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(
        batch.records(),
        &[json!({"id": 1}), json!({"id": 2}), json!({"id": 3})]
    );
}

/// Test that LogWriter rolls to a new segment when max_file_size is exceeded.
#[tokio::test]
async fn test_writer_rolls_on_size() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Set a very small max_file_size so each record triggers a roll
    let mut writer = LogWriterConfig::new(log_dir.clone())
        .compression_level(3)
        .max_file_size(1) // 1 byte — every write will exceed this
        .build();

    writer.write_value(&json!({"id": 1})).unwrap();
    writer.write_value(&json!({"id": 2})).unwrap();
    writer.close().unwrap();

    // Should have created 2 segment files
    let segments: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    k9::assert_equal!(segments.len(), 2);

    // Both should be readonly (done)
    for seg in &segments {
        assert!(
            seg.metadata().unwrap().permissions().readonly(),
            "{:?} should be readonly",
            seg.file_name()
        );
    }

    // Tailer should read both in order
    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let mut all = Vec::new();
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            batch = tailer.next() => {
                match batch {
                    Some(Ok(b)) => {
                        all.extend(b.records().iter().cloned());
                        if all.len() >= 2 { break; }
                    }
                    Some(Err(e)) => panic!("error: {e}"),
                    None => break,
                }
            }
            _ = &mut timeout => panic!("timed out"),
        }
    }
    k9::assert_equal!(all, vec![json!({"id": 1}), json!({"id": 2})]);
}

/// Test that LogWriter respects the suffix option.
#[tokio::test]
async fn test_writer_suffix() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    let mut writer = LogWriterConfig::new(log_dir.clone())
        .compression_level(3)
        .max_file_size(10_000)
        .suffix(".zst")
        .build();

    writer.write_value(&json!({"x": 1})).unwrap();
    writer.close().unwrap();

    let files: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    k9::assert_equal!(files.len(), 1);
    assert!(
        files[0].ends_with(".zst"),
        "expected .zst suffix, got {}",
        files[0]
    );
}
