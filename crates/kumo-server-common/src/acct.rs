use crate::authn_authz::{Access, AuditRecord, AuthInfo, Identity};
use crate::disk_space::{MinFree, MonitoredPath};
use crate::log::{mark_existing_logs_as_done_in_dir, OpenedFile};
use anyhow::Context;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::task::JoinHandle;
use zstd::Encoder;

static LOGGER: OnceLock<AcctLogger> = OnceLock::new();

#[derive(Debug)]
enum AcctCommand {
    Record(AcctLogRecord),
    /// Stop after writing every record queued ahead of this command.
    Terminate,
}

struct AcctLogger {
    tx: Sender<AcctCommand>,
    /// Present until `shutdown` takes it to join the logger task.
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct AuditLogParams {
    /// Where to place the log files
    pub log_dir: PathBuf,
    /// How many uncompressed bytes to allow per file segment
    #[serde(default = "AuditLogParams::default_max_file_size")]
    pub max_file_size: u64,
    /// Maximum number of outstanding items to be logged before
    /// the submission will block; helps to avoid runaway issues
    /// spiralling out of control.
    #[serde(default = "AuditLogParams::default_back_pressure")]
    pub back_pressure: usize,

    /// The level of compression.
    /// 0 - use the zstd default level (probably 3).
    /// 1-21 are the explicitly configurable levels
    #[serde(default = "AuditLogParams::default_compression_level")]
    pub compression_level: i32,

    #[serde(default, with = "duration_serde")]
    pub max_segment_duration: Option<Duration>,

    #[serde(default)]
    pub min_free_space: MinFree,
    #[serde(default)]
    pub min_free_inodes: MinFree,

    /// Log records for successfully granted authz
    #[serde(default = "AuditLogParams::default_true")]
    pub log_authz_allow: bool,

    /// Log records for denied authz
    #[serde(default = "AuditLogParams::default_true")]
    pub log_authz_deny: bool,

    /// Log records for successful authn
    #[serde(default = "AuditLogParams::default_true")]
    pub log_authn_ok: bool,

    /// Log records for failed authn
    #[serde(default = "AuditLogParams::default_true")]
    pub log_authn_fail: bool,
}

impl AuditLogParams {
    pub fn default_max_file_size() -> u64 {
        1_000_000_000
    }
    pub fn default_back_pressure() -> usize {
        128_000
    }
    pub fn default_compression_level() -> i32 {
        0 // use the zstd default
    }

    pub fn default_true() -> bool {
        true
    }

    pub async fn init(&self) -> anyhow::Result<()> {
        let (tx, rx) = channel(self.back_pressure);
        let params = self.clone();
        let task = kumo_server_runtime::spawn_on_main("acct logger", async move {
            if let Err(err) = params.acct_logger(rx).await {
                tracing::error!(
                    "AuditLogParams::acct_logger returned with error: {err}, \
                    audit logging will cease to function until the next restart!"
                );
            }
        })?;

        let logger = AcctLogger {
            tx,
            task: Mutex::new(Some(task)),
        };
        if LOGGER.set(logger).is_err() {
            anyhow::bail!("AuditLogParams::init must not be called more than once");
        }

        Ok(())
    }

    async fn acct_logger(self, mut rx: Receiver<AcctCommand>) -> anyhow::Result<()> {
        if !config::is_validating() {
            if let Err(err) = mark_existing_logs_as_done_in_dir(&self.log_dir) {
                tracing::error!("{err}");
            }
            std::fs::create_dir_all(&self.log_dir).ok();
            MonitoredPath {
                name: format!("log dir {}", self.log_dir.display()),
                path: self.log_dir.clone(),
                min_free_space: self.min_free_space,
                min_free_inodes: self.min_free_inodes,
            }
            .register();
        }

        let mut expire_counter = 0u16;
        let mut current_file: Option<OpenedFile> = None;

        loop {
            let deadline = current_file.as_ref().and_then(|f| f.expires);

            let mut check_expire = false;
            let record = match deadline {
                Some(deadline) => tokio::select! {
                    cmd = rx.recv() => {
                        match cmd {
                            Some(AcctCommand::Record(r)) => Some(r),
                            Some(AcctCommand::Terminate) | None => return Ok(()),
                        }
                    }
                    _ = tokio::time::sleep_until(deadline.into()) => {
                        check_expire = true;
                        None
                    }
                },
                None => match rx.recv().await {
                    Some(AcctCommand::Record(r)) => Some(r),
                    Some(AcctCommand::Terminate) | None => return Ok(()),
                },
            };

            expire_counter = expire_counter.wrapping_add(1);
            if expire_counter == 10_000 {
                check_expire = true;
            }

            if check_expire {
                let now = Instant::now();
                let do_expire = current_file
                    .as_ref()
                    .and_then(|f| f.expires.map(|exp| exp <= now))
                    .unwrap_or(false);

                if do_expire {
                    current_file.take();
                }
                expire_counter = 0;
            }

            if let Some(record) = record {
                let should_log = match &record {
                    AcctLogRecord::Authentication(authn) => {
                        if authn.success {
                            self.log_authn_ok
                        } else {
                            self.log_authn_fail
                        }
                    }
                    AcctLogRecord::Authorization(authz) => match authz.access {
                        Access::Allow => self.log_authz_allow,
                        Access::Deny => self.log_authz_deny,
                    },
                };

                if !should_log {
                    continue;
                }

                if let Err(err) = self.open_file_if_needed(&mut current_file) {
                    tracing::error!("{err}");
                    continue;
                }

                if let Err(err) = self.do_record(&mut current_file, record) {
                    tracing::error!("{err}");
                    continue;
                }
            }
        }
    }

    fn open_file_if_needed(&self, current_file: &mut Option<OpenedFile>) -> anyhow::Result<()> {
        if current_file.is_some() {
            return Ok(());
        }
        let now = Utc::now();

        let base_name = now.format("acct-%Y%m%d-%H%M%S%.f").to_string();

        let name = self.log_dir.join(base_name);
        // They might be trying to use multiple directories below
        // the configured log_dir, so adjust our idea of its parent
        // dir
        let log_dir = name
            .parent()
            .expect("log_dir.join ensures we always have a parent");

        let f = match std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&name)
        {
            Ok(f) => f,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    match std::fs::create_dir_all(log_dir) {
                        Ok(_) => {
                            // Try opening it again now
                            std::fs::OpenOptions::new()
                                .append(true)
                                .create(true)
                                .open(&name)
                                .with_context(|| format!("open log file {name:?}"))?
                        }
                        Err(dir_err) => {
                            anyhow::bail!(
                                "open log file {name:?}: failed: {err:#?}. \
                                    Additionally, attempting to create dir {} failed: {dir_err:#?}",
                                log_dir.display()
                            );
                        }
                    }
                } else {
                    anyhow::bail!("open log file {name:?}: failed: {err:#?}");
                }
            }
        };

        let file = OpenedFile {
            file: Encoder::new(f, self.compression_level).context("set up zstd encoder")?,
            name,
            written: 0,
            expires: self
                .max_segment_duration
                .map(|duration| Instant::now() + duration),
        };

        current_file.replace(file);
        Ok(())
    }

    fn do_record(
        &self,
        current_file: &mut Option<OpenedFile>,
        record: AcctLogRecord,
    ) -> anyhow::Result<()> {
        if let Some(file) = current_file {
            let mut record_text = Vec::new();
            serde_json::to_writer(&mut record_text, &record).context("serializing record")?;
            record_text.push(b'\n');

            file.file
                .write_all(&record_text)
                .with_context(|| format!("writing record to {}", file.name.display()))?;
            file.written += record_text.len() as u64;

            let need_rotate = file.written >= self.max_file_size
                || file
                    .expires
                    .map(|exp| exp <= Instant::now())
                    .unwrap_or(false);

            if need_rotate {
                current_file.take();
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum AcctLogRecord {
    Authentication(AuthnAuditRecord),
    Authorization(AuditRecord),
}

impl AcctLogRecord {
    pub fn timestamp(&self) -> &DateTime<Utc> {
        match self {
            Self::Authentication(authn) => &authn.timestamp,
            Self::Authorization(authz) => &authz.timestamp,
        }
    }

    pub fn is_allow(&self) -> bool {
        match self {
            Self::Authentication(authn) => authn.success,
            Self::Authorization(authz) => authz.access == Access::Allow,
        }
    }
}

async fn log_acct(record: AcctLogRecord) -> anyhow::Result<()> {
    tracing::trace!("Audit: {record:?}");
    if let Some(logger) = LOGGER.get() {
        logger.tx.send(AcctCommand::Record(record)).await?;
    }
    Ok(())
}

/// Stop the audit logger and finalize its open log segment. Records
/// submitted before this call are written first. Blocks until the
/// logger task has exited. Does nothing if the logger was never
/// configured.
pub async fn shutdown() {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    logger.tx.send(AcctCommand::Terminate).await.ok();
    let task = logger.task.lock().take();
    if let Some(task) = task {
        if let Err(err) = task.await {
            tracing::error!("acct logger task failed: {err:#}");
        }
    }
}

pub async fn log_authz(record: AuditRecord) -> anyhow::Result<()> {
    log_acct(AcctLogRecord::Authorization(record)).await
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AuthnAuditRecord {
    /// When the event occurred
    pub timestamp: DateTime<Utc>,
    /// Who they were attempting to authenticate as
    pub attempted_identity: Identity,
    /// Whether the attempt was successful
    pub success: bool,
    /// The resulting authentication info
    pub auth_info: AuthInfo,
}

pub async fn log_authn(record: AuthnAuditRecord) -> anyhow::Result<()> {
    log_acct(AcctLogRecord::Authentication(record)).await
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::authn_authz::IdentityContext;

    /// Builds params that log everything into `log_dir`, with no time based
    /// rotation and the given size limit.
    fn params(log_dir: &std::path::Path, max_file_size: u64) -> AuditLogParams {
        AuditLogParams {
            log_dir: log_dir.to_path_buf(),
            max_file_size,
            back_pressure: 16,
            compression_level: AuditLogParams::default_compression_level(),
            max_segment_duration: None,
            min_free_space: MinFree::default(),
            min_free_inodes: MinFree::default(),
            log_authz_allow: true,
            log_authz_deny: true,
            log_authn_ok: true,
            log_authn_fail: true,
        }
    }

    fn authn_record(identity: &str) -> AcctLogRecord {
        AcctLogRecord::Authentication(AuthnAuditRecord {
            timestamp: Utc::now(),
            attempted_identity: Identity {
                identity: identity.to_string(),
                context: IdentityContext::SmtpAuthPlainAuthentication,
            },
            success: true,
            auth_info: AuthInfo::default(),
        })
    }

    /// Decode every segment in `dir`, returning the identity from each
    /// record in filename order.
    fn read_identities(dir: &std::path::Path) -> anyhow::Result<Vec<Vec<String>>> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        paths.sort();
        paths
            .iter()
            .map(|path| {
                let data = zstd::stream::decode_all(std::fs::File::open(path)?)?;
                String::from_utf8(data)?
                    .lines()
                    .map(|line| match serde_json::from_str(line)? {
                        AcctLogRecord::Authentication(authn) => {
                            Ok(authn.attempted_identity.identity)
                        }
                        other => anyhow::bail!("unexpected record {other:?}"),
                    })
                    .collect()
            })
            .collect()
    }

    /// With no size or time based rotation, shutdown is required to complete
    /// the zstd frame.
    #[tokio::test]
    async fn shutdown_finalizes_open_segment() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        params(dir.path(), AuditLogParams::default_max_file_size())
            .init()
            .await?;

        let AcctLogRecord::Authentication(record) = authn_record("daniel") else {
            unreachable!()
        };
        log_authn(record).await?;

        shutdown().await;

        k9::assert_equal!(
            read_identities(dir.path())?,
            vec![vec!["daniel".to_string()]]
        );
        Ok(())
    }

    #[test]
    fn size_limit_rotates_segment() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let params = params(dir.path(), 1);
        let mut current_file = None;

        for identity in ["first", "second"] {
            params.open_file_if_needed(&mut current_file)?;
            params.do_record(&mut current_file, authn_record(identity))?;
            assert!(
                current_file.is_none(),
                "segment must rotate after {identity}"
            );
        }

        k9::assert_equal!(
            read_identities(dir.path())?,
            vec![vec!["first".to_string()], vec!["second".to_string()]]
        );
        Ok(())
    }
}
