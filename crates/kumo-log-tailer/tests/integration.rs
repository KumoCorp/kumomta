use camino::Utf8PathBuf;
use futures::StreamExt;
use kumo_log_tailer::{LogBatch, LogTailer, LogTailerConfig};
use serde_json::json;
use std::io::Write;
use std::time::Duration;
use tempfile::TempDir;

/// Helper: create a zstd-compressed JSONL file with the given records.
/// If `mark_done` is true, sets the file to readonly (simulating writer completion).
fn write_zstd_log(dir: &std::path::Path, filename: &str, records: &[&str], mark_done: bool) {
    let path = dir.join(filename);
    let mut encoder = zstd::Encoder::new(std::fs::File::create(&path).unwrap(), 3).unwrap();
    for record in records {
        writeln!(encoder, "{record}").unwrap();
    }
    encoder.finish().unwrap();

    if mark_done {
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();
    }
}

fn utf8_dir(dir: &TempDir) -> Utf8PathBuf {
    Utf8PathBuf::try_from(dir.path().to_path_buf()).unwrap()
}

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

    write_zstd_log(dir.path(), "segment-001.zst", &records, true);

    let mut all_records = Vec::new();

    // Read one record at a time, closing and reopening with checkpoint
    for i in 0..5 {
        let tailer = LogTailerConfig::new(log_dir.clone())
            .pattern("*.zst")
            .max_batch_size(1)
            .max_batch_latency(Duration::from_millis(50))
            .checkpoint_name("test-cp")
            .build()
            .await
            .unwrap();

        tokio::pin!(tailer);

        let batch = tailer.next().await;
        let mut batch = batch
            .unwrap_or_else(|| panic!("expected a batch on iteration {i}"))
            .unwrap_or_else(|e| panic!("expected Ok batch on iteration {i}: {e}"));
        k9::assert_equal!(batch.len(), 1);
        all_records.push(batch.records()[0].clone());
        batch.commit().unwrap();

        tailer.as_mut().close();
    }

    // Verify we got all records exactly once, in order
    let expected: Vec<serde_json::Value> = records
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    k9::assert_equal!(all_records, expected);

    // One more tailer should yield no records from the completed file
    // (it should just wait). Close it immediately.
    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
        .max_batch_size(1)
        .max_batch_latency(Duration::from_millis(50))
        .checkpoint_name("test-cp")
        .build()
        .await
        .unwrap();

    // close immediately; should get None
    tailer.close();
    tokio::pin!(tailer);
    let result = tailer.next().await;
    k9::assert_equal!(result.is_none(), true);
}

#[tokio::test]
async fn test_multiple_files_in_order() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_zstd_log(dir.path(), "aaa.zst", &[r#"{"file":"a","n":1}"#], true);
    write_zstd_log(
        dir.path(),
        "bbb.zst",
        &[r#"{"file":"b","n":1}"#, r#"{"file":"b","n":2}"#],
        true,
    );
    write_zstd_log(dir.path(), "ccc.zst", &[r#"{"file":"c","n":1}"#], true);

    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();

    tokio::pin!(tailer);

    // Collect all records from completed files
    let mut all_records = Vec::new();
    // We expect to get batches covering all 4 records from 3 files.
    // The tailer may yield them in one or more batches.
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

#[tokio::test]
async fn test_checkpoint_across_multiple_files() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[r#"{"id":1}"#, r#"{"id":2}"#],
        true,
    );
    write_zstd_log(
        dir.path(),
        "seg-002.zst",
        &[r#"{"id":3}"#, r#"{"id":4}"#],
        true,
    );

    let mut all_records = Vec::new();

    // Read one at a time across two files
    for i in 0..4 {
        let tailer = LogTailerConfig::new(log_dir.clone())
            .pattern("*.zst")
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

    let expected = vec![
        json!({"id":1}),
        json!({"id":2}),
        json!({"id":3}),
        json!({"id":4}),
    ];
    k9::assert_equal!(all_records, expected);
}

/// Verify that commit() after reading one record advances the checkpoint
/// so the next tailer sees the *next* record, not the same one.
#[tokio::test]
async fn test_commit_advances_checkpoint_past_consumed_batch() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    write_zstd_log(
        dir.path(),
        "data.zst",
        &[r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#],
        true,
    );

    // First tailer: read one record, then close
    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
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

    // Second tailer with same checkpoint: should see record 2, not record 1
    let tailer2 = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
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

    write_zstd_log(
        dir.path(),
        "data.zst",
        &[r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#],
        true,
    );

    // First tailer: read one record, then drop without close
    {
        let tailer = LogTailerConfig::new(log_dir.clone())
            .pattern("*.zst")
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

    // Second tailer with same checkpoint: should see record 1 again
    // because commit() was never called.
    let tailer2 = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
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
    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[r#"{"seg":1,"n":1}"#, r#"{"seg":1,"n":2}"#],
        true,
    );
    write_zstd_log(
        dir.path(),
        "seg-002.zst",
        &[r#"{"seg":2,"n":1}"#, r#"{"seg":2,"n":2}"#],
        true,
    );

    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
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

    // Should only contain records from seg-002, not seg-001
    k9::assert_equal!(
        all_records,
        vec![json!({"seg": 2, "n": 1}), json!({"seg": 2, "n": 2})]
    );

    // Tail mode must NOT have created any checkpoint file.
    // Since no checkpoint_name was set, no file should exist.
    // Check that the directory contains only our two log segments.
    let mut dir_entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    dir_entries.sort();
    k9::assert_equal!(dir_entries, vec!["seg-001.zst", "seg-002.zst"]);
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

/// Verify that a single batch can contain records from multiple segment
/// files when the batch size is large enough to span both.
#[tokio::test]
async fn test_batch_spans_multiple_segments() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Two small completed segments: 2 records + 2 records = 4 total.
    // With batch_size=10 they should all land in one batch.
    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[r#"{"seg":1,"n":1}"#, r#"{"seg":1,"n":2}"#],
        true,
    );
    write_zstd_log(
        dir.path(),
        "seg-002.zst",
        &[r#"{"seg":2,"n":1}"#, r#"{"seg":2,"n":2}"#],
        true,
    );

    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
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

    // First two lines come from seg-001, last two from seg-002
    let seg1 = log_dir.join("seg-001.zst");
    let seg2 = log_dir.join("seg-002.zst");
    k9::assert_equal!(batch.file_name_for_line(0), &seg1);
    k9::assert_equal!(batch.file_name_for_line(1), &seg1);
    k9::assert_equal!(batch.file_name_for_line(2), &seg2);
    k9::assert_equal!(batch.file_name_for_line(3), &seg2);
}

/// Verify that max_batch_size still constrains the batch even when
/// multiple segments are available. Records beyond the limit should
/// appear in subsequent batches.
#[tokio::test]
async fn test_batch_size_constrains_across_segments() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Three records across two segments, but batch size is 2.
    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[r#"{"seg":1,"n":1}"#, r#"{"seg":1,"n":2}"#],
        true,
    );
    write_zstd_log(dir.path(), "seg-002.zst", &[r#"{"seg":2,"n":1}"#], true);

    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
        .max_batch_size(2)
        .max_batch_latency(Duration::from_millis(100))
        .build()
        .await
        .unwrap();
    tokio::pin!(tailer);

    // First batch: exactly 2 records (the limit), all from seg-001
    let batch1 = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch1.len(), 2);
    k9::assert_equal!(
        batch1.records(),
        &[json!({"seg": 1, "n": 1}), json!({"seg": 1, "n": 2}),]
    );
    k9::assert_equal!(batch1.file_names().len(), 1);

    // Second batch: the remaining record from seg-002
    let batch2 = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch2.len(), 1);
    k9::assert_equal!(batch2.records(), &[json!({"seg": 2, "n": 1})]);
    k9::assert_equal!(batch2.file_names().len(), 1);

    let seg2 = log_dir.join("seg-002.zst");
    k9::assert_equal!(batch2.file_name_for_line(0), &seg2);
}

/// Verify that a partial batch (fewer records than max_batch_size) is
/// yielded after max_batch_latency expires when the file is still
/// being written to (not yet marked readonly).
#[tokio::test]
async fn test_partial_batch_flushed_by_latency() {
    let dir = TempDir::new().unwrap();
    let log_dir = utf8_dir(&dir);

    // Write a file with 3 records but do NOT mark it done.
    // The tailer will read the 3 records, hit EOF, and wait for more
    // data.  After max_batch_latency it should yield a partial batch.
    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#],
        false, // not done
    );

    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
        .max_batch_size(100) // much larger than available records
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
        &[json!({"n": 1}), json!({"n": 2}), json!({"n": 3}),]
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

    // Write a completed file with fewer records than max_batch_size.
    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[r#"{"n":1}"#, r#"{"n":2}"#],
        true, // done
    );

    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
        .max_batch_size(100)
        .max_batch_latency(Duration::from_secs(10)) // very long
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

    // Write the first segment and mark it done.
    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[r#"{"n":1}"#, r#"{"n":2}"#],
        true,
    );

    let mut config = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
        .max_batch_size(100)
        .max_batch_latency(Duration::from_millis(100));
    if let Some(interval) = poll_watcher {
        config = config.poll_watcher(interval);
    }
    let tailer = config.build().await.unwrap();
    tokio::pin!(tailer);

    // Read the first batch — should contain both records from seg-001.
    let batch1 = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch1.records(), &[json!({"n": 1}), json!({"n": 2})]);

    // Now the tailer is waiting for new files.  Give it a moment
    // to enter the wait state, then write a second segment.
    tokio::time::sleep(Duration::from_millis(200)).await;

    write_zstd_log(
        dir.path(),
        "seg-002.zst",
        &[r#"{"n":3}"#, r#"{"n":4}"#],
        true,
    );

    // The tailer should discover the new file and yield its records.
    let batch2 = next_batch_with_timeout(&mut tailer).await;
    k9::assert_equal!(batch2.records(), &[json!({"n": 3}), json!({"n": 4})]);

    // Verify no duplicates: collect everything and check order.
    let mut all: Vec<serde_json::Value> = Vec::new();
    all.extend(batch1.records().iter().cloned());
    all.extend(batch2.records().iter().cloned());
    k9::assert_equal!(
        all,
        vec![
            json!({"n": 1}),
            json!({"n": 2}),
            json!({"n": 3}),
            json!({"n": 4}),
        ]
    );
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

    write_zstd_log(dir.path(), "seg-001.zst", &[r#"{"n":1}"#], true);

    // No checkpoint_name configured
    let tailer = LogTailerConfig::new(log_dir.clone())
        .pattern("*.zst")
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

    // No checkpoint file should have been created
    let mut dir_entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    dir_entries.sort();
    k9::assert_equal!(dir_entries, vec!["seg-001.zst"]);
}
