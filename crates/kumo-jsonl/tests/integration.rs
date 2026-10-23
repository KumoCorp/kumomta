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

/// Return the path of the log segment in `dir`, ignoring the dot-prefixed
/// checkpoint files.
fn segment_path(dir: &std::path::Path) -> Utf8PathBuf {
    let mut segs: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(".")
                .starts_with('.')
        })
        .collect();
    segs.sort();
    Utf8PathBuf::try_from(segs.pop().expect("a segment file")).unwrap()
}

/// Write checkpoint `.name` in `log_dir` with position `line` in `file`.
fn write_checkpoint(log_dir: &Utf8PathBuf, name: &str, file: &Utf8PathBuf, line: usize) {
    let data = json!({"file": file.as_str(), "line": line});
    std::fs::write(
        log_dir.join(format!(".{name}")),
        serde_json::to_vec(&data).unwrap(),
    )
    .unwrap();
}

/// Read the `line` field of checkpoint `.name` in `log_dir`, or `None`
/// if the checkpoint file does not exist.
fn read_checkpoint_line(log_dir: &Utf8PathBuf, name: &str) -> Option<usize> {
    let bytes = std::fs::read(log_dir.join(format!(".{name}"))).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    Some(v["line"].as_u64().unwrap() as usize)
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

/// Mark `path` readonly.  Both successful `LogWriter::close()` and the
/// producer's startup sweep over abandoned segments do this; tests use
/// it to simulate the latter.
fn mark_readonly(path: &std::path::Path) {
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(path, perms).unwrap();
}

/// A killed writer leaves the zstd stream un-finished.  When the
/// producer restarts, its startup sweep marks the abandoned segment
/// readonly.  The tailer must then yield the recoverable records and
/// advance past the truncated tail, not error out.  Simulated by
/// dropping the writer without `finish` and then marking the file
/// readonly to mimic the sweep.
#[tokio::test]
async fn test_truncated_segment_skips_partial_trailing_data() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // First (truncated) segment: 5 records, no zstd finish.
    write_open_segment(
        dir.path(),
        &[
            r#"{"seg":1,"n":1}"#,
            r#"{"seg":1,"n":2}"#,
            r#"{"seg":1,"n":3}"#,
            r#"{"seg":1,"n":4}"#,
            r#"{"seg":1,"n":5}"#,
        ],
    );
    // Find that segment and mark readonly to simulate the producer's
    // startup sweep marking an abandoned segment as done.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    k9::assert_equal!(entries.len(), 1);
    let first_seg = entries[0].path();
    mark_readonly(&first_seg);

    // Second (clean) segment afterwards.
    write_segment(dir.path(), &[r#"{"seg":2,"n":1}"#]);

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    // Collect everything that comes through; we don't assert on
    // exactly how many records from seg 1 made it (depends on zstd
    // block boundaries) but we MUST eventually see seg 2's record
    // and we MUST NOT see an Err.
    let mut saw_seg2 = false;
    let mut seg1_count = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let next = tokio::time::timeout(remaining, tailer.next()).await;
        match next {
            Ok(Some(Ok(batch))) => {
                for rec in batch.records() {
                    match rec.get("seg").and_then(|v| v.as_u64()) {
                        Some(1) => seg1_count += 1,
                        Some(2) => saw_seg2 = true,
                        _ => panic!("unexpected record {rec}"),
                    }
                }
                if saw_seg2 {
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("tailer must not error on truncated segment: {e}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    assert!(saw_seg2, "tailer never advanced past the truncated segment");
    assert!(
        seg1_count > 0,
        "expected some records from the truncated segment to be recovered"
    );
}

/// A non-zstd foreign file dropped into the log directory must not
/// stop the tailer.  Surrounding legitimate segments must still be
/// read.
#[tokio::test]
async fn test_foreign_non_zstd_file_is_skipped() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Legitimate segment first.
    write_segment(dir.path(), &[r#"{"n":1}"#]);
    // Foreign file with a name that sorts between/around segments.
    std::fs::write(dir.path().join("notes.txt"), b"hello, this is not zstd\n").unwrap();
    // Another legitimate segment.
    write_segment(dir.path(), &[r#"{"n":2}"#]);

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let mut all = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while all.len() < 2 && tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(remaining, tailer.next()).await {
            Ok(Some(Ok(b))) => all.extend(b.records().iter().cloned()),
            Ok(Some(Err(e))) => panic!("tailer must not error on foreign file: {e}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    k9::assert_equal!(all, vec![json!({"n": 1}), json!({"n": 2})]);
}

/// A valid zstd file whose decompressed contents are not JSONL must
/// not stop the tailer.
#[tokio::test]
async fn test_foreign_non_jsonl_zstd_file_is_skipped() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Legitimate segment first.
    write_segment(dir.path(), &[r#"{"n":1}"#]);
    // Foreign zstd file containing non-JSON text.
    let compressed = zstd::stream::encode_all(&b"this is not json at all\n"[..], 3).unwrap();
    std::fs::write(dir.path().join("foreign.zst"), compressed).unwrap();
    // Another legitimate segment.
    write_segment(dir.path(), &[r#"{"n":2}"#]);

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let mut all = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while all.len() < 2 && tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(remaining, tailer.next()).await {
            Ok(Some(Ok(b))) => all.extend(b.records().iter().cloned()),
            Ok(Some(Err(e))) => panic!("tailer must not error on foreign zstd file: {e}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    k9::assert_equal!(all, vec![json!({"n": 1}), json!({"n": 2})]);
}

/// A bad file whose name sorts after future legitimate segments must
/// not poison `last_processed` and hide those segments.
#[tokio::test]
async fn test_bad_file_sorting_after_future_segments_does_not_block() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // The writer names files with a date-stamped prefix, so a foreign
    // file named with a `z` prefix will sort after any future segment.
    std::fs::write(dir.path().join("zzz-foreign.txt"), b"junk\n").unwrap();

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    // Drain whatever surfaces from the foreign file -- there should be
    // no batches, and definitely no error.  Do a brief poll.
    let _ = tokio::time::timeout(Duration::from_millis(300), tailer.next()).await;

    // Now write a legitimate segment.  Its name (a timestamp) sorts
    // before "zzz-foreign.txt", so if `last_processed` had been set to
    // the foreign file we would never see this record.
    write_segment(dir.path(), &[r#"{"n":42}"#]);

    let batch = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch.records(), &[json!({"n": 42})]);
}

/// A record larger than the `max_line_size` of the tailer is discarded, but the
/// records around it in the same segment are still delivered rather than the
/// whole segment being dropped.
#[tokio::test]
async fn test_oversized_record_skipped_rest_of_segment_read() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // The writer uses its default (large) max_record_size and writes the big
    // record unmodified. Only this tailer is configured with a small
    // max_line_size, which is what rejects the record.
    let big = "x".repeat(256 * 1024);
    write_segment(
        dir.path(),
        &[r#"{"id":"before"}"#, &big, r#"{"id":"after"}"#],
    );

    let tailer = LogTailerConfig::new(log_dir.clone())
        .max_batch_size(10)
        .max_batch_latency(Duration::from_millis(50))
        .max_line_size(64 * 1024)
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    let batch = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(
        batch.records(),
        &[json!({"id": "before"}), json!({"id": "after"})]
    );
}

/// Ensure a consumer whose filter drops every record still records durable
/// progress: once it has scanned a completed segment its checkpoint advances to
/// the end of that segment to avoid re-scanning the same records after a
/// restart.
#[tokio::test]
async fn test_filtered_consumer_advances_checkpoint() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_segment(
        dir.path(),
        &[
            r#"{"n":1}"#,
            r#"{"n":2}"#,
            r#"{"n":3}"#,
            r#"{"n":4}"#,
            r#"{"n":5}"#,
        ],
    );

    let sink = ConsumerConfig::new("sink")
        .checkpoint_name("filter-cp")
        .filter(|_record| Ok(false));

    let tailer = MultiConsumerTailerConfig::new(log_dir.clone(), vec![sink])
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    // Bound the wait to keep this test from hanging while the tailer waits for
    // new records.
    let timeout = tokio::time::sleep(Duration::from_millis(500));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            b = tailer.next() => match b {
                Some(Ok(batches)) => {
                    panic!("unexpected batch from filter-all consumer: {} batches", batches.len())
                }
                Some(Err(e)) => panic!("unexpected error: {e}"),
                None => break,
            },
            _ = &mut timeout => break,
        }
    }
    tailer.as_mut().close();

    // The checkpoint must have advanced past all five scanned records.
    let cp_bytes = std::fs::read(log_dir.join(".filter-cp")).expect("checkpoint should exist");
    let cp: serde_json::Value = serde_json::from_slice(&cp_bytes).unwrap();
    k9::assert_equal!(cp["line"], json!(5));
}

/// Verify that a consumer with an uncommitted delivered batch retains its
/// checkpoint at or before that batch when flushing progress from filtered
/// records. After a restart the uncommitted records are re-read.
#[tokio::test]
async fn test_filtered_flush_preserves_uncommitted_records() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_segment(
        dir.path(),
        &[r#"{"keep":true}"#, r#"{"keep":false}"#, r#"{"keep":false}"#],
    );

    {
        let selective = ConsumerConfig::new("selective")
            .max_batch_size(1)
            .checkpoint_name("select-cp")
            .filter(|record| Ok(record["keep"].as_bool().unwrap_or(false)));

        let tailer = MultiConsumerTailerConfig::new(log_dir.clone(), vec![selective])
            .build()
            .await
            .unwrap();
        tokio::pin!(tailer);

        let timeout = tokio::time::sleep(Duration::from_millis(500));
        tokio::pin!(timeout);
        let mut saw_match = false;
        loop {
            tokio::select! {
                b = tailer.next() => match b {
                    Some(Ok(batches)) => {
                        k9::assert_equal!(batches[0].records(), &[json!({"keep": true})]);
                        saw_match = true;
                    }
                    Some(Err(e)) => panic!("unexpected error: {e}"),
                    None => break,
                },
                _ = &mut timeout => break,
            }
        }
        assert!(saw_match, "expected the matching record to be delivered");
        tailer.as_mut().close();
    }

    let cp_line = read_checkpoint_line(&log_dir, "select-cp");
    assert!(
        matches!(cp_line, None | Some(0)),
        "checkpoint advanced to {cp_line:?} despite an uncommitted batch"
    );

    let all = ConsumerConfig::new("all")
        .max_batch_size(10)
        .checkpoint_name("select-cp")
        .filter(|_record| Ok(true));
    let tailer = MultiConsumerTailerConfig::new(log_dir.clone(), vec![all])
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);
    let timeout = tokio::time::sleep(Duration::from_millis(500));
    tokio::pin!(timeout);
    let mut records = Vec::new();
    loop {
        tokio::select! {
            b = tailer.next() => match b {
                Some(Ok(batches)) => {
                    for batch in &batches {
                        records.extend(batch.records().iter().cloned());
                    }
                }
                Some(Err(e)) => panic!("unexpected error: {e}"),
                None => break,
            },
            _ = &mut timeout => break,
        }
    }
    tailer.as_mut().close();
    k9::assert_equal!(
        records,
        vec![
            json!({"keep": true}),
            json!({"keep": false}),
            json!({"keep": false})
        ]
    );
}

/// Verifies that a dropped, never-committed batch does not permanently block
/// flushing progress for filtered records: once a later batch commits past it,
/// the flush resumes and the checkpoint advances over the filtered records that
/// follow.
#[tokio::test]
async fn test_filtered_flush_resumes_after_dropped_batch() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Two matching records, then a filtered-out tail.
    write_segment(
        dir.path(),
        &[
            r#"{"keep":true}"#,
            r#"{"keep":true}"#,
            r#"{"keep":false}"#,
            r#"{"keep":false}"#,
            r#"{"keep":false}"#,
        ],
    );

    {
        let selective = ConsumerConfig::new("selective")
            .max_batch_size(1)
            .max_batch_latency(Duration::from_millis(50))
            .checkpoint_name("resume-cp")
            .filter(|record| Ok(record["keep"].as_bool().unwrap_or(false)));
        let tailer = MultiConsumerTailerConfig::new(log_dir.clone(), vec![selective])
            .build()
            .await
            .unwrap();
        tokio::pin!(tailer);

        let timeout = tokio::time::sleep(Duration::from_millis(500));
        tokio::pin!(timeout);
        let mut match_count = 0;
        loop {
            tokio::select! {
                b = tailer.next() => match b {
                    Some(Ok(mut batches)) => {
                        match_count += 1;
                        if match_count > 1 {
                            for batch in &mut batches {
                                batch.commit().unwrap();
                            }
                        }
                    }
                    Some(Err(e)) => panic!("unexpected error: {e}"),
                    None => break,
                },
                _ = &mut timeout => break,
            }
        }
        assert!(
            match_count >= 2,
            "expected both matching records, saw {match_count}"
        );
        tailer.as_mut().close();
    }

    let cp_line = read_checkpoint_line(&log_dir, "resume-cp");
    k9::assert_equal!(cp_line, Some(5));
}

/// On restart, a consumer checkpoint can be ahead of the current scan position.
/// Flushing progress for records rejected by filters preserves that later
/// checkpoint.
#[tokio::test]
async fn test_filtered_flush_does_not_regress_ahead_consumer() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_open_segment(dir.path(), &[r#"{"n":0}"#, r#"{"n":1}"#, r#"{"n":2}"#]);
    let seg = segment_path(dir.path());

    write_checkpoint(&log_dir, "ahead-cp", &seg, 5);
    write_checkpoint(&log_dir, "behind-cp", &seg, 1);

    let ahead = ConsumerConfig::new("ahead")
        .checkpoint_name("ahead-cp")
        .filter(|_r| Ok(false));
    let behind = ConsumerConfig::new("behind")
        .checkpoint_name("behind-cp")
        .filter(|_r| Ok(false));

    let tailer = MultiConsumerTailerConfig::new(log_dir.clone(), vec![ahead, behind])
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    // Let the tailer scan to the end of the open segment and flush.
    let timeout = tokio::time::sleep(Duration::from_millis(500));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            b = tailer.next() => match b {
                Some(Ok(batches)) => panic!("unexpected batch: {} batches", batches.len()),
                Some(Err(e)) => panic!("unexpected error: {e}"),
                None => break,
            },
            _ = &mut timeout => break,
        }
    }
    tailer.as_mut().close();

    k9::assert_equal!(read_checkpoint_line(&log_dir, "behind-cp"), Some(3));
    // The checkpoint of the ahead consumer must remain at or above its resume
    // line.
    k9::assert_equal!(read_checkpoint_line(&log_dir, "ahead-cp"), Some(5));
}
