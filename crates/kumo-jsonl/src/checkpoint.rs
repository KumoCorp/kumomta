use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::time::{Duration, SystemTime};

/// Prefix reserved for checkpoint files whose atomic writes are still in
/// progress.
pub const CHECKPOINT_TEMP_PREFIX: &str = ".tmp.checkpoint.";

/// Returns whether `name` would produce a checkpoint file that collides with
/// the reserved temporary-write namespace. Such a name cannot be used because a
/// directory scan would treat its checkpoint as an in-progress write and ignore
/// it.
pub fn is_reserved_checkpoint_name(name: &str) -> bool {
    // The 1.. here is because the constant starts with `.` but the
    // `name` parameter here doesn't have a `.` prefix
    name.starts_with(&CHECKPOINT_TEMP_PREFIX[1..])
}

/// Age past which a leftover checkpoint temp file is treated as abandoned. A
/// committed write renames its temp file into place within microseconds of
/// creating it. A temp file older than this was never renamed, which means the
/// process that created it exited before finishing the write.
pub const CHECKPOINT_TEMP_MAX_AGE: Duration = Duration::from_secs(300);

/// Remove checkpoint temp files in `directory` left behind by a process that
/// crashed between writing and renaming a checkpoint. Only entries older than
/// `max_age` are removed, leaving a temp file currently being written by
/// another consumer process untouched. Best-effort: a missing directory or a
/// per-entry failure is skipped rather than reported.
pub fn sweep_orphaned_temp_files(directory: &Utf8Path, max_age: Duration) {
    let Ok(read_dir) = std::fs::read_dir(directory.as_std_path()) else {
        return;
    };
    let now = SystemTime::now();
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(CHECKPOINT_TEMP_PREFIX) {
            continue;
        }
        let is_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= max_age);
        if is_old {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => {}
                // We treat not found as success because the goal of the sweep
                // is an absent file.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => tracing::warn!(
                    "removing orphaned checkpoint temp file {:?}: {err:#}",
                    entry.path()
                ),
            }
        }
    }
}

/// Persisted checkpoint data recording the current file and line position.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct CheckpointData {
    pub file: String,
    pub line: usize,
}

impl CheckpointData {
    /// Load a checkpoint from the given path.
    /// Returns `Ok(None)` if the file does not exist.
    pub async fn load(path: &Utf8PathBuf) -> anyhow::Result<Option<Self>> {
        match tokio::fs::read(path.as_std_path()).await {
            Ok(bytes) => {
                let data: Self = serde_json::from_slice(&bytes)?;
                Ok(Some(data))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Atomically write a checkpoint file by writing to a temporary
    /// file in the same directory and then renaming it into place.
    pub fn save_atomic(
        checkpoint_path: &Utf8PathBuf,
        file: &Utf8PathBuf,
        line: usize,
    ) -> anyhow::Result<()> {
        let data = Self {
            file: file.to_string(),
            line,
        };
        let json = serde_json::to_string(&data)?;
        let dir = checkpoint_path
            .parent()
            .unwrap_or_else(|| Utf8Path::new("."));
        let mut tmp = tempfile::Builder::new()
            .prefix(CHECKPOINT_TEMP_PREFIX)
            .tempfile_in(dir.as_std_path())?;
        tmp.write_all(json.as_bytes())?;
        tmp.persist(checkpoint_path.as_std_path())?;
        Ok(())
    }
}
