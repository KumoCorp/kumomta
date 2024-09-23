use filenamegen::Glob;
use parking_lot::FairMutex as Mutex;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::sync::watch::{channel, Receiver, Sender};
use tokio::task::spawn_blocking;

static CONFIG: LazyLock<Mutex<ConfigurationParams>> =
    LazyLock::new(|| Mutex::new(ConfigurationParams::new()));

static EPOCH: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfigEpoch(usize);

pub struct ConfigurationParams {
    pub globs: Arc<Vec<Glob>>,
    sender: Sender<ConfigEpoch>,
    receiver: Receiver<ConfigEpoch>,
}

impl ConfigurationParams {
    pub fn new() -> Self {
        let glob = Glob::new("/opt/kumomta/etc/**/*.{lua,json,toml,yaml}")
            .expect("failed to parse default glob");

        let (sender, receiver) = channel(ConfigEpoch(0));
        Self {
            globs: Arc::new(vec![glob]),
            sender,
            receiver,
        }
    }

    pub fn set_globs(&mut self, patterns: Vec<String>) -> anyhow::Result<()> {
        let mut globs = vec![];
        for p in patterns {
            globs.push(Glob::new(&p)?);
        }
        self.globs = Arc::new(globs);
        Ok(())
    }

    async fn eval_globs() -> anyhow::Result<BTreeSet<PathBuf>> {
        let globs = CONFIG.lock().globs.clone();
        spawn_blocking(move || {
            let mut paths: BTreeSet<PathBuf> = BTreeSet::new();
            for glob in globs.iter() {
                for path in glob.walk("/") {
                    let path = if !path.is_absolute() {
                        Path::new("/").join(path)
                    } else {
                        path
                    };
                    if path.is_file() {
                        paths.insert(path);
                    }
                }
            }
            Ok(paths)
        })
        .await?
    }

    pub fn subscribe(&self) -> Receiver<ConfigEpoch> {
        self.receiver.clone()
    }

    async fn config_epoch_task() -> anyhow::Result<()> {
        tracing::info!("config_epoch_task: starting");

        pub fn compute_hash(paths: BTreeSet<PathBuf>) -> String {
            if paths.is_empty() {
                tracing::warn!("config_epoch_task: glob evaluated to no paths");
                return "-no-files-".into();
            }

            let mut ctx = Sha256::new();
            for path in &paths {
                tracing::trace!("hashing {}", path.display());
                ctx.update(path.display().to_string());
                match std::fs::File::open(path) {
                    Ok(mut f) => {
                        let mut buf = [0u8; 8192];
                        while let Ok(n) = f.read(&mut buf) {
                            if n == 0 {
                                break;
                            }
                            ctx.update(&buf[0..n]);
                        }
                    }
                    Err(err) => {
                        tracing::error!("Error opening {}: {err:#}", path.display());
                    }
                }
            }
            let hash = ctx.finalize();
            let hex = data_encoding::HEXLOWER.encode(&hash);

            tracing::debug!("hashed {} files as {hex}", paths.len());

            hex
        }

        let mut current_hash = String::new();

        loop {
            match Self::eval_globs().await {
                Ok(paths) => match spawn_blocking(move || compute_hash(paths)).await {
                    Ok(hash) => {
                        if hash != current_hash {
                            tracing::info!("config_epoch_task: config change detected {hash:?}");
                            current_hash = hash.clone();

                            bump_current_epoch();
                        }
                    }
                    Err(err) => {
                        tracing::error!("config_epoch_task: error computing epoch: {err:#}");
                    }
                },
                Err(err) => {
                    tracing::error!("config_epoch_task: error computing hashes: {err:#}");
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
    }
}

pub fn bump_current_epoch() {
    let epoch = 1 + EPOCH.fetch_add(1, Ordering::SeqCst);
    CONFIG.lock().sender.send(ConfigEpoch(epoch)).ok();
}

pub fn get_current_epoch() -> ConfigEpoch {
    ConfigEpoch(EPOCH.load(Ordering::SeqCst))
}

pub fn subscribe() -> Receiver<ConfigEpoch> {
    CONFIG.lock().subscribe()
}

pub fn set_globs(globs: Vec<String>) -> anyhow::Result<()> {
    CONFIG.lock().set_globs(globs)
}

pub async fn eval_globs() -> anyhow::Result<Vec<String>> {
    let mut result = vec![];
    let paths = ConfigurationParams::eval_globs().await?;
    for p in paths {
        result.push(
            p.to_str()
                .ok_or_else(|| anyhow::anyhow!("path {} cannot be converted to UTF8", p.display()))?
                .to_string(),
        );
    }
    Ok(result)
}

pub fn start_monitor() {
    tokio::spawn(async move {
        if let Err(err) = ConfigurationParams::config_epoch_task().await {
            tracing::error!("config_epoch_task: {err:#}");
        }
    });
}
