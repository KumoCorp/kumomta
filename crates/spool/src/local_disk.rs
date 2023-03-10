use crate::{Spool, SpoolEntry, SpoolId};
use anyhow::Context;
use async_trait::async_trait;
use std::fs::File;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tokio::sync::mpsc::Sender;

pub struct LocalDiskSpool {
    path: PathBuf,
    flush: bool,
    _pid_file: File,
}

impl LocalDiskSpool {
    pub fn new(path: &Path, flush: bool) -> anyhow::Result<Self> {
        let pid_file_path = path.join("lock");
        let _pid_file = lock_pid_file(pid_file_path)?;

        Self::create_dir_structure(path)?;

        Ok(Self {
            path: path.to_path_buf(),
            flush,
            _pid_file,
        })
    }

    fn create_dir_structure(path: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(path.join("new"))?;
        std::fs::create_dir_all(path.join("data"))?;
        Ok(())
    }

    fn compute_path(&self, id: SpoolId) -> PathBuf {
        id.compute_path(&self.path.join("data"))
    }

    fn cleanup_dirs(path: &Path) {
        let new_dir = path.join("new");
        for entry in jwalk::WalkDir::new(new_dir) {
            if let Ok(entry) = entry {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                if let Err(err) = std::fs::remove_file(&path) {
                    eprintln!("Failed to remove {path:?}: {err:#}");
                }
            }
        }

        let data_dir = path.join("data");
        Self::cleanup_data(&data_dir);
    }

    fn cleanup_data(data_dir: &Path) {
        for entry in jwalk::WalkDir::new(data_dir) {
            if let Ok(entry) = entry {
                if !entry.file_type().is_dir() {
                    continue;
                }
                let path = entry.path();
                // Speculatively try removing the directory; it will
                // only succeed if it is empty. We don't need to check
                // for that first, and we don't care if it fails.
                std::fs::remove_dir(&path).ok();
            }
        }
    }
}

#[async_trait]
impl Spool for LocalDiskSpool {
    async fn load(&self, id: SpoolId) -> anyhow::Result<Vec<u8>> {
        let path = self.compute_path(id);
        tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed to load {id} from {path:?}"))
    }

    async fn remove(&self, id: SpoolId) -> anyhow::Result<()> {
        let path = self.compute_path(id);
        tokio::fs::remove_file(&path)
            .await
            .with_context(|| format!("failed to remove {id} from {path:?}"))
    }

    async fn store(&self, id: SpoolId, data: &[u8]) -> anyhow::Result<()> {
        let path = self.compute_path(id);
        let new_dir = self.path.join("new");
        let data = data.to_vec();
        let flush = self.flush;
        tokio::task::Builder::new()
            .name("LocalDiskSpool store")
            .spawn_blocking(move || {
                let mut temp = NamedTempFile::new_in(new_dir)
                    .with_context(|| format!("failed to create a temporary file to store {id}"))?;

                temp.write_all(&data)
                    .with_context(|| format!("failed to write data for {id}"))?;

                if flush {
                    temp.as_file_mut()
                        .sync_data()
                        .with_context(|| format!("failed to sync data for {id}"))?;
                }

                std::fs::create_dir_all(path.parent().unwrap())
                    .with_context(|| format!("failed to create dir structure for {id} {path:?}"))?;

                temp.persist(&path)
                    .with_context(|| format!("failed to move temp file for {id} to {path:?}"))?;
                Ok(())
            })?
            .await?
    }

    fn enumerate(&self, sender: Sender<SpoolEntry>) -> anyhow::Result<()> {
        let path = self.path.clone();
        tokio::task::Builder::new()
            .name("LocalDiskSpool enumerate")
            .spawn_blocking(move || -> anyhow::Result<()> {
                Self::cleanup_dirs(&path);

                for entry in jwalk::WalkDir::new(path.join("data")) {
                    if let Ok(entry) = entry {
                        if !entry.file_type().is_file() {
                            continue;
                        }
                        let path = entry.path();
                        if let Some(id) = SpoolId::from_path(&path) {
                            match std::fs::read(&path) {
                                Ok(data) => sender
                                    .blocking_send(SpoolEntry::Item { id, data })
                                    .map_err(|err| {
                                        anyhow::anyhow!("failed to send data for {id}: {err:#}")
                                    })?,
                                Err(err) => sender
                                    .blocking_send(SpoolEntry::Corrupt {
                                        id,
                                        error: format!("{err:#}"),
                                    })
                                    .map_err(|err| {
                                        anyhow::anyhow!(
                                            "failed to send SpoolEntry for {id}: {err:#}"
                                        )
                                    })?,
                            };
                        } else {
                            eprintln!("{} is not a spool id", path.display());
                        }
                    }
                }
                anyhow::Result::Ok(())
            })?;
        Ok(())
    }

    async fn cleanup(&self) -> anyhow::Result<()> {
        let data_dir = self.path.join("data");
        Ok(tokio::task::Builder::new()
            .name("LocalDiskSpool cleanup")
            .spawn_blocking(move || {
                Self::cleanup_data(&data_dir);
            })?
            .await?)
    }
}

/// Set the sticky bit on path.
/// This prevents tmpwatch from removing the lock file.
pub fn set_sticky_bit(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = path.metadata() {
            let mut perms = metadata.permissions();
            let mode = perms.mode();
            perms.set_mode(mode | libc::S_ISVTX as u32);
            let _ = std::fs::set_permissions(&path, perms);
        }
    }

    #[cfg(windows)]
    {
        let _ = path;
    }
}

fn lock_pid_file(pid_file: PathBuf) -> anyhow::Result<std::fs::File> {
    let pid_file_dir = pid_file
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent?", pid_file.display()))?;
    std::fs::create_dir_all(&pid_file_dir).with_context(|| {
        format!(
            "while creating directory structure: {}",
            pid_file_dir.display()
        )
    })?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&pid_file)
        .with_context(|| format!("opening pid file {}", pid_file.display()))?;
    set_sticky_bit(&pid_file);
    let res = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if res != 0 {
        let err = std::io::Error::last_os_error();

        let owner = match std::fs::read_to_string(&pid_file) {
            Ok(pid) => format!(". Owned by pid {}.", pid.trim()),
            Err(_) => "".to_string(),
        };

        anyhow::bail!(
            "unable to lock pid file {}: {}{owner}",
            pid_file.display(),
            err
        );
    }

    unsafe { libc::ftruncate(file.as_raw_fd(), 0) };
    writeln!(file, "{}", unsafe { libc::getpid() }).ok();

    Ok(file)
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn basic_spool() -> anyhow::Result<()> {
        let location = tempfile::tempdir()?;
        let spool = LocalDiskSpool::new(&location.path(), false)?;
        let data_dir = location.path().join("data");

        {
            let id1 = SpoolId::new();
            let id1_path = id1.compute_path(&data_dir).display().to_string();

            // Can't load an entry that doesn't exist
            assert_eq!(
                format!("{:#}", spool.load(id1).await.unwrap_err()),
                format!(
                    "failed to load {id1} from \"{id1_path}\": \
                    No such file or directory (os error 2)"
                )
            );
        }

        // Insert some entries
        let mut ids = vec![];
        for i in 0..100 {
            let id = SpoolId::new();
            spool.store(id, format!("I am {i}").as_bytes()).await?;
            ids.push(id);
        }

        // Verify that we can load those entries
        for (i, &id) in ids.iter().enumerate() {
            let data = spool.load(id).await?;
            let text = String::from_utf8(data)?;
            assert_eq!(text, format!("I am {i}"));
        }

        {
            // Verify that we can enumerate them
            let (tx, mut rx) = tokio::sync::mpsc::channel(32);
            spool.enumerate(tx)?;
            let mut count = 0;

            while let Some(item) = rx.recv().await {
                match item {
                    SpoolEntry::Item { id, data } => {
                        let i = ids
                            .iter()
                            .position(|&item| item == id)
                            .ok_or_else(|| anyhow::anyhow!("{id} not found in ids!"))?;

                        let text = String::from_utf8(data)?;
                        assert_eq!(text, format!("I am {i}"));

                        spool.remove(id).await?;
                        // Can't load an entry that we just removed
                        let id_path = id.compute_path(&data_dir).display().to_string();
                        assert_eq!(
                            format!("{:#}", spool.load(id).await.unwrap_err()),
                            format!(
                                "failed to load {id} from \"{id_path}\": \
                                No such file or directory (os error 2)"
                            )
                        );
                        count += 1;
                    }
                    SpoolEntry::Corrupt { id, error } => {
                        anyhow::bail!("Corrupt: {id}: {error}");
                    }
                }
            }

            assert_eq!(count, 100);
        }

        // Now that we've removed the files, try enumerating again.
        // We expect to receive no entries.
        // Do it a couple of times to verify that none of the cleanup
        // stuff that happens in enumerate breaks the directory
        // structure
        for _ in 0..2 {
            // Verify that we can enumerate them
            let (tx, mut rx) = tokio::sync::mpsc::channel(32);
            spool.enumerate(tx)?;
            let mut unexpected = vec![];

            while let Some(item) = rx.recv().await {
                match item {
                    SpoolEntry::Item { id, .. } | SpoolEntry::Corrupt { id, .. } => {
                        unexpected.push(id)
                    }
                }
            }

            assert_eq!(unexpected.len(), 0);
        }

        Ok(())
    }
}
