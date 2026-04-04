use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::io::Write;

/// Persisted checkpoint data recording the current file and line position.
#[derive(Deserialize, Serialize, Debug, Clone)]
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
            .map(|p| p.as_std_path())
            .unwrap_or_else(|| std::path::Path::new("."));
        let prefix = checkpoint_path.file_name().unwrap_or(".checkpoint");
        let mut tmp = tempfile::Builder::new().prefix(prefix).tempfile_in(dir)?;
        tmp.write_all(json.as_bytes())?;
        tmp.persist(checkpoint_path.as_std_path())?;
        Ok(())
    }
}
