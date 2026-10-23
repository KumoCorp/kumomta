use camino::Utf8PathBuf;
use kumo_jsonl::backlog::{scan_backlog, ConsumerBacklog};
use kumo_jsonl::checkpoint::{
    is_reserved_checkpoint_name, sweep_orphaned_temp_files, CheckpointData, CHECKPOINT_TEMP_PREFIX,
};
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;

fn utf8_dir(dir: &TempDir) -> Utf8PathBuf {
    Utf8PathBuf::try_from(dir.path().to_path_buf()).unwrap()
}

/// Assert a report matches `expected` in full, except for `lag`: the caller
/// states only whether it should be present. Its exact value depends on
/// wall-clock time elapsed since the test created the segment and varies
/// between runs.
fn assert_backlog(actual: &ConsumerBacklog, lag_present: bool, expected: ConsumerBacklog) {
    k9::assert_equal!(actual.lag.is_some(), lag_present);
    k9::assert_equal!(
        actual,
        &ConsumerBacklog {
            lag: actual.lag,
            ..expected
        }
    );
}

/// Create a segment file of `size` bytes, filled with `b'x'`. The content
/// does not matter.
fn write_segment(dir: &Utf8PathBuf, name: &str, size: usize) -> Utf8PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, vec![b'x'; size]).unwrap();
    path
}

/// Write a checkpoint file `.name` that records `file` and `line`.
fn write_checkpoint(dir: &Utf8PathBuf, name: &str, file: &Utf8PathBuf, line: usize) {
    let data = json!({"file": file.as_str(), "line": line});
    std::fs::write(
        dir.join(format!(".{name}")),
        serde_json::to_vec(&data).unwrap(),
    )
    .unwrap();
}

/// Verifies that a consumer that has reached the newest segment has an empty
/// backlog.
#[test]
fn caught_up() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    write_segment(&dir, "seg-1", 10);
    write_segment(&dir, "seg-2", 20);
    let head = write_segment(&dir, "seg-3", 30);
    write_checkpoint(&dir, "consumer", &head, 5);

    let reports = scan_backlog(&dir, "*").unwrap();
    k9::assert_equal!(reports.len(), 1);
    assert_backlog(
        &reports[0],
        false,
        ConsumerBacklog {
            checkpoint_name: "consumer".to_string(),
            checkpoint: CheckpointData {
                file: head.to_string(),
                line: 5,
            },
            segments_behind: 0,
            bytes_behind: 0,
            lag: None,
        },
    );
}

/// Verifies that a consumer trailing several segments is reported with the
/// count and total size of the segments that follow its own current segment.
#[test]
fn behind_by_segments() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    let first = write_segment(&dir, "seg-1", 10);
    write_segment(&dir, "seg-2", 20);
    write_segment(&dir, "seg-3", 30);
    write_checkpoint(&dir, "consumer", &first, 3);

    let reports = scan_backlog(&dir, "*").unwrap();
    k9::assert_equal!(reports.len(), 1);
    assert_backlog(
        &reports[0],
        true,
        ConsumerBacklog {
            checkpoint_name: "consumer".to_string(),
            checkpoint: CheckpointData {
                file: first.to_string(),
                line: 3,
            },
            segments_behind: 2,
            // Combined size of seg-2 and seg-3. seg-1 holds the checkpoint and
            // is excluded.
            bytes_behind: 50,
            lag: None,
        },
    );
}

/// Verifies that a checkpoint still matches its segment when its recorded path
/// differs cosmetically (a redundant "./", a different base directory) from the
/// path produced by the scan.
#[test]
fn checkpoint_path_differs_cosmetically() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    write_segment(&dir, "seg-1", 10);
    write_segment(&dir, "seg-2", 20);

    // The checkpoint stored seg-1 via a path with a redundant "./" and a
    // trailing slash on the directory, unlike the clean path produced by the
    // scan.
    let cosmetic = Utf8PathBuf::from(format!("{dir}/./seg-1"));
    write_checkpoint(&dir, "consumer", &cosmetic, 3);

    let reports = scan_backlog(&dir, "*").unwrap();
    k9::assert_equal!(reports.len(), 1);
    assert_backlog(
        &reports[0],
        true,
        ConsumerBacklog {
            checkpoint_name: "consumer".to_string(),
            checkpoint: CheckpointData {
                file: cosmetic.to_string(),
                line: 3,
            },
            segments_behind: 1,
            bytes_behind: 20,
            lag: None,
        },
    );
}

/// Consumers are discovered from their checkpoint files and reported in
/// name order.
#[test]
fn discovers_multiple_consumers() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    let first = write_segment(&dir, "seg-1", 10);
    let head = write_segment(&dir, "seg-2", 20);
    write_checkpoint(&dir, "behind", &first, 1);
    write_checkpoint(&dir, "ahead", &head, 1);

    let reports = scan_backlog(&dir, "*").unwrap();
    k9::assert_equal!(reports.len(), 2);
    assert_backlog(
        &reports[0],
        false,
        ConsumerBacklog {
            checkpoint_name: "ahead".to_string(),
            checkpoint: CheckpointData {
                file: head.to_string(),
                line: 1,
            },
            segments_behind: 0,
            bytes_behind: 0,
            lag: None,
        },
    );
    assert_backlog(
        &reports[1],
        true,
        ConsumerBacklog {
            checkpoint_name: "behind".to_string(),
            checkpoint: CheckpointData {
                file: first.to_string(),
                line: 1,
            },
            segments_behind: 1,
            bytes_behind: 20,
            lag: None,
        },
    );
}

/// Verifies that an in-progress checkpoint write (a reserved-prefix dotfile)
/// and an unrelated dotfile are both excluded from the report.
#[test]
fn ignores_temp_writes_and_stray_dotfiles() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    let head = write_segment(&dir, "seg-1", 10);
    write_checkpoint(&dir, "consumer", &head, 1);

    std::fs::write(
        dir.join(format!("{CHECKPOINT_TEMP_PREFIX}abcd12")),
        json!({"file": "seg-1", "line": 9}).to_string(),
    )
    .unwrap();
    // A stray dotfile that is non-checkpoint JSON.
    std::fs::write(dir.join(".DS_Store"), b"not json").unwrap();

    let reports = scan_backlog(&dir, "*").unwrap();
    k9::assert_equal!(reports.len(), 1);
    assert_backlog(
        &reports[0],
        false,
        ConsumerBacklog {
            checkpoint_name: "consumer".to_string(),
            checkpoint: CheckpointData {
                file: head.to_string(),
                line: 1,
            },
            segments_behind: 0,
            bytes_behind: 0,
            lag: None,
        },
    );
}

/// Verifies that a checkpoint whose segment has been rotated away still counts
/// the segments that remain ahead of it.
#[test]
fn checkpoint_segment_rotated_away() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    write_segment(&dir, "seg-2", 20);
    write_segment(&dir, "seg-3", 30);
    // Use a missing segment to cover checkpoints retained after segment rotation.
    let gone = dir.join("seg-1");
    write_checkpoint(&dir, "consumer", &gone, 7);

    let reports = scan_backlog(&dir, "*").unwrap();
    k9::assert_equal!(reports.len(), 1);
    assert_backlog(
        &reports[0],
        true,
        ConsumerBacklog {
            checkpoint_name: "consumer".to_string(),
            checkpoint: CheckpointData {
                file: gone.to_string(),
                line: 7,
            },
            segments_behind: 2,
            bytes_behind: 50,
            lag: None,
        },
    );
}

/// Verifies that a checkpoint written through `save_atomic` is discovered as
/// one consumer: its temp file is gone by the time a scan can see it.
#[test]
fn save_atomic_checkpoint_is_discovered() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    let head = write_segment(&dir, "seg-1", 10);
    CheckpointData::save_atomic(&dir.join(".consumer"), &head, 4).unwrap();

    let reports = scan_backlog(&dir, "*").unwrap();
    k9::assert_equal!(reports.len(), 1);
    assert_backlog(
        &reports[0],
        false,
        ConsumerBacklog {
            checkpoint_name: "consumer".to_string(),
            checkpoint: CheckpointData {
                file: head.to_string(),
                line: 4,
            },
            segments_behind: 0,
            bytes_behind: 0,
            lag: None,
        },
    );
}

/// Verifies that scanning a non-existent directory yields an empty report
/// rather than an error, since a per-record log directory may not be created
/// yet.
#[test]
fn missing_directory_is_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp).join("not-created-yet");
    let reports = scan_backlog(&dir, "*").unwrap();
    k9::assert_equal!(reports.len(), 0);
}

/// A checkpoint name is rejected when its file would collide with the reserved
/// temporary-write prefix.
#[test]
fn reserved_checkpoint_names() {
    assert!(is_reserved_checkpoint_name("tmp.checkpoint.foo"));
    assert!(!is_reserved_checkpoint_name("deliveries"));
    assert!(!is_reserved_checkpoint_name("tmp"));
}

/// The sweep removes abandoned temp files while leaving committed
/// checkpoints and segments in place.  A zero `max_age` treats every
/// temp file as abandoned.
#[test]
fn sweep_removes_orphaned_temp_files() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    write_segment(&dir, "seg-1", 10);
    let head = write_segment(&dir, "seg-2", 10);
    write_checkpoint(&dir, "consumer", &head, 1);
    std::fs::write(
        dir.join(format!("{CHECKPOINT_TEMP_PREFIX}abcd12")),
        b"partial",
    )
    .unwrap();

    sweep_orphaned_temp_files(&dir, Duration::ZERO);

    assert!(!dir.join(format!("{CHECKPOINT_TEMP_PREFIX}abcd12")).exists());
    assert!(dir.join("seg-1").exists());
    assert!(dir.join("seg-2").exists());
    assert!(dir.join(".consumer").exists());
}

/// Verifies that the sweep does not remove a temp file younger than
/// `max_age`, because a write that recent may still be in flight.
#[test]
fn sweep_keeps_recent_temp_files() {
    let tmp = TempDir::new().unwrap();
    let dir = utf8_dir(&tmp);
    let temp = dir.join(format!("{CHECKPOINT_TEMP_PREFIX}inflight"));
    std::fs::write(&temp, b"partial").unwrap();

    sweep_orphaned_temp_files(&dir, Duration::from_secs(3600));

    assert!(temp.exists());
}
