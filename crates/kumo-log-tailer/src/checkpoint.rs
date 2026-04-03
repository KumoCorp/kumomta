use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

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

    /// Save a checkpoint to the given path.
    pub async fn save(
        checkpoint_path: &Utf8PathBuf,
        file: &Utf8PathBuf,
        line: usize,
    ) -> anyhow::Result<()> {
        let data = Self {
            file: file.to_string(),
            line,
        };
        let json = serde_json::to_string(&data)?;
        tokio::fs::write(checkpoint_path.as_std_path(), json).await?;
        Ok(())
    }
}
