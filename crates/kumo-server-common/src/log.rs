use std::fs::File;
use std::path::PathBuf;
use std::time::Instant;
use zstd::stream::write::Encoder;

/// Represents an opened log file
pub struct OpenedFile {
    pub file: Encoder<'static, File>,
    pub name: PathBuf,
    pub written: u64,
    pub expires: Option<Instant>,
}

impl Drop for OpenedFile {
    fn drop(&mut self) {
        self.file.do_finish().ok();
        mark_path_as_done(&self.name).ok();
        tracing::debug!("Flushed {:?}", self.name);
    }
}

pub fn mark_path_as_done(path: &PathBuf) -> std::io::Result<()> {
    let meta = path.metadata()?;
    // Remove the `w` bit to signal to the tailer that this
    // file will not be written to any more and that it is
    // now considered to be complete
    let mut perms = meta.permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(path, perms)
}

pub fn mark_existing_logs_as_done_in_dir(dir: &PathBuf) -> anyhow::Result<()> {
    match std::fs::read_dir(dir) {
        Ok(d) => {
            for entry in d {
                if let Ok(entry) = entry {
                    match entry.file_name().to_str() {
                        Some(name) if name.starts_with('.') => {
                            continue;
                        }
                        None => continue,
                        Some(_name) => {
                            if let Ok(file_type) = entry.file_type() {
                                if file_type.is_file() {
                                    mark_path_as_done(&entry.path()).ok();
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                // If there's no dir, there's nothing to mark done!
                Ok(())
            } else {
                anyhow::bail!(
                    "failed to mark existing logs as done in {}: {err:#}",
                    dir.display()
                );
            }
        }
    }
}
