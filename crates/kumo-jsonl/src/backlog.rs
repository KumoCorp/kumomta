//! Observe, from outside the consumer processes, how far each consumer trails
//! the newest data in a log directory.

use crate::checkpoint::{CheckpointData, CHECKPOINT_TEMP_PREFIX};
use camino::{Utf8Path, Utf8PathBuf};
use filenamegen::Glob;
use serde::Serialize;
use std::time::{Duration, SystemTime};

/// How far one consumer's checkpoint trails the newest data in the log
/// directory.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConsumerBacklog {
    /// The `checkpoint_name` with which the consumer was configured
    pub checkpoint_name: String,
    /// Where the consumer's reading has reached.
    pub checkpoint: CheckpointData,
    /// Count of whole segments ordered after the segment defined by `checkpoint`.
    pub segments_behind: usize,
    /// Total compressed size, in bytes, of the segments counted by
    /// `segments_behind`. Excludes the remainder of the segment that contains
    /// the checkpoint, because measuring that would require decompressing it to
    /// find the recorded line.
    pub bytes_behind: u64,
    /// Age of the oldest segment not started by the consumer, measured from its
    /// modification time, approximating how long the oldest unread data has
    /// waited. `None` when the consumer has reached the newest segment.
    pub lag: Option<Duration>,
}

/// Scan `directory` and report the backlog of each consumer relative to the
/// newest segment matching `pattern`. Consumers are discovered by their
/// `.<checkpoint_name>` files. Segment contents are never decompressed. Only
/// their directory listing and file metadata are read. A nonexistent directory
/// yields an empty report.
pub fn scan_backlog(directory: &Utf8Path, pattern: &str) -> anyhow::Result<Vec<ConsumerBacklog>> {
    let segments = sorted_segments(directory, pattern)?;

    let read_dir = match std::fs::read_dir(directory.as_std_path()) {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(err) => return Err(err.into()),
    };

    let mut reports = vec![];
    for entry in read_dir {
        // An unreadable or racily-removed entry must not abort the whole scan.
        // Skip it.
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        // In-progress checkpoint writes are dotfiles too. The reserved prefix
        // distinguishes them from committed checkpoints.
        if name.starts_with(CHECKPOINT_TEMP_PREFIX) {
            continue;
        }
        let Some(checkpoint_name) = name.strip_prefix('.') else {
            continue;
        };
        // Includes only parseable dotfiles, excluding editor swap files and
        // stray dotfiles.
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(checkpoint) = serde_json::from_slice::<CheckpointData>(&bytes) else {
            continue;
        };

        reports.push(backlog_for(
            checkpoint_name.to_string(),
            checkpoint,
            &segments,
        ));
    }

    reports.sort_by(|a, b| a.checkpoint_name.cmp(&b.checkpoint_name));
    Ok(reports)
}

/// Caches the segment metadata needed to calculate backlog without reading
/// segment contents.
struct Segment {
    path: Utf8PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

/// Return the sorted segments in `directory` matching `pattern`, excluding the
/// dotfiles that hold checkpoints and in-progress writes.
fn sorted_segments(directory: &Utf8Path, pattern: &str) -> anyhow::Result<Vec<Segment>> {
    let glob = Glob::new(pattern)?;
    let mut segments = vec![];
    for path in glob.walk(directory) {
        // A non-UTF-8 name cannot be one of our segments. Skip it rather than
        // aborting the scan.
        let Ok(rel) = Utf8PathBuf::try_from(path) else {
            continue;
        };
        let path = directory.join(&rel);
        if path.file_name().is_some_and(|n| n.starts_with('.')) {
            continue;
        }
        let metadata = match path.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => continue,
            // We exclude a segment we can't stat from bytes_behind rather
            // than guess its size, trusting an undercount over a fabricated
            // one.
            Err(err) => {
                tracing::debug!("skipping log segment {path}: {err:#}");
                continue;
            }
        };
        segments.push(Segment {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            path,
        });
    }
    segments.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(segments)
}

/// Derive the backlog for a checkpoint against the sorted segments.
fn backlog_for(
    checkpoint_name: String,
    checkpoint: CheckpointData,
    segments: &[Segment],
) -> ConsumerBacklog {
    // Use the filename as the segment identity because equivalent directory
    // paths can differ in absolute form, symlink resolution, or trailing slash.
    let cp_name = Utf8Path::new(&checkpoint.file)
        .file_name()
        .unwrap_or(checkpoint.file.as_str());
    // The segments that sort strictly after the checkpoint segment remain
    // unread by the consumer. Comparing filenames, rather than requiring the
    // checkpoint segment to still be present in `segments`, keeps this correct
    // even after that segment has been rotated away.
    let later = segments
        .iter()
        .filter(|s| s.path.file_name().is_some_and(|n| n > cp_name));

    let mut segments_behind = 0;
    let mut bytes_behind = 0;
    let mut oldest_unread: Option<&Segment> = None;
    for seg in later {
        segments_behind += 1;
        bytes_behind += seg.len;
        if oldest_unread.is_none() {
            oldest_unread = Some(seg);
        }
    }

    let lag = oldest_unread.and_then(|seg| SystemTime::now().duration_since(seg.modified?).ok());

    ConsumerBacklog {
        checkpoint_name,
        checkpoint,
        segments_behind,
        bytes_behind,
        lag,
    }
}
