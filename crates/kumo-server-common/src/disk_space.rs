use anyhow::Context;
use human_bytes::human_bytes;
use num_format::{Locale, ToFormattedString};
use prometheus::IntGaugeVec;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, Once};
use std::time::Duration;

static OVER_LIMIT: AtomicBool = AtomicBool::new(false);
static PATHS: LazyLock<Mutex<Vec<MonitoredPath>>> = LazyLock::new(Default::default);
static MONITOR: Once = Once::new();
static FREE_INODES: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "disk_free_inodes",
        "number of available inodes in a monitored location",
        &["name"]
    )
    .unwrap()
});
static FREE_INODES_PCT: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "disk_free_inodes_percent",
        "percentage of available inodes in a monitored location",
        &["name"]
    )
    .unwrap()
});
static FREE_SPACE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "disk_free_bytes",
        "number of available bytes in a monitored location",
        &["name"]
    )
    .unwrap()
});
static FREE_SPACE_PCT: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "disk_free_percent",
        "percentage of available bytes in a monitored location",
        &["name"]
    )
    .unwrap()
});

#[derive(Hash, PartialEq, Eq, Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "serde_json::Value")]
pub enum MinFree {
    Percent(u8),
    Value(u64),
}

impl TryFrom<Value> for MinFree {
    type Error = String;
    fn try_from(v: Value) -> Result<Self, String> {
        match v {
            Value::String(s) => {
                match s.strip_suffix("%") {
                    Some(n) => n
                        .parse::<u8>()
                        .map_err(|err| format!("invalid MinFree percentage specifier. {err:#}"))
                        .map(|n| MinFree::Percent(n)),
                    None => s
                        .parse::<u64>()
                        .map_err(|err| format!("invalid MinFree size specifier. {err:#}"))
                        .map(|n| MinFree::Value(n)),
                }
            }
            Value::Number(n) => {
                match n.as_u64() {
                    Some(n) => Ok(MinFree::Value(n)),
                    None => Err(format!("invalid MinFree size specifier. {n} could not be converted to u64"))
                }
            }
            v => Err(format!("invalid MinFree specifier {v:?}. Value must be either a percentage string like '10%' or the size expressed as an integer"))
        }
    }
}

impl TryFrom<String> for MinFree {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::try_from(s.as_str())
    }
}

impl TryFrom<&str> for MinFree {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s.strip_suffix("%") {
            Some(n) => n
                .parse::<u8>()
                .map_err(|err| format!("invalid MinFree percentage specifier. {err:#}"))
                .map(|n| MinFree::Percent(n)),
            None => s
                .parse::<u64>()
                .map_err(|err| format!("invalid MinFree size specifier. {err:#}"))
                .map(|n| MinFree::Value(n)),
        }
    }
}

impl Default for MinFree {
    fn default() -> Self {
        Self::Percent(10)
    }
}

#[derive(Hash, PartialEq, Eq, Debug, Clone)]
pub struct MonitoredPath {
    pub name: String,
    pub path: PathBuf,
    pub min_free_space: MinFree,
    pub min_free_inodes: MinFree,
}

pub struct AvailableSpace {
    pub space_avail: u64,
    pub space_avail_percent: u8,
    pub inodes_avail: u64,
    pub inodes_avail_percent: u8,
}

impl MonitoredPath {
    pub fn register(self) {
        PATHS.lock().unwrap().push(self);

        MONITOR.call_once(|| {
            std::thread::Builder::new()
                .name("disk-space-monitor".to_string())
                .spawn(monitor_thread)
                .expect("failed to spawn disk-space-monitor thread");
        });
    }

    pub fn get_usage(&self) -> anyhow::Result<AvailableSpace> {
        let info = nix::sys::statvfs::statvfs(&self.path)
            .with_context(|| format!("statvfs({}) failed", self.path.display()))?;

        let blocks_avail = info.blocks_available() as u64;
        let blocks_total = info.blocks() as u64;

        let space_avail_percent =
            ((blocks_avail as f64 / blocks_total as f64) * 100.0).floor() as u8;
        let space_avail = blocks_avail * info.block_size();
        FREE_SPACE
            .get_metric_with_label_values(&[&self.name])
            .unwrap()
            .set(space_avail as i64);
        FREE_SPACE_PCT
            .get_metric_with_label_values(&[&self.name])
            .unwrap()
            .set(space_avail_percent as i64);

        let inodes_avail = info.files_available() as u64;
        let inodes_total = info.files();
        let inodes_avail_percent =
            ((inodes_avail as f64 / inodes_total as f64) * 100.0).floor() as u8;
        FREE_INODES
            .get_metric_with_label_values(&[&self.name])
            .unwrap()
            .set(inodes_avail as i64);
        FREE_INODES_PCT
            .get_metric_with_label_values(&[&self.name])
            .unwrap()
            .set(inodes_avail_percent as i64);

        Ok(AvailableSpace {
            space_avail,
            space_avail_percent,
            inodes_avail,
            inodes_avail_percent,
        })
    }

    pub fn check_usage(&self, avail: &AvailableSpace) -> anyhow::Result<()> {
        let mut reason = vec![];

        match self.min_free_space {
            MinFree::Percent(p) if avail.space_avail_percent < p => {
                reason.push(format!(
                    "{}% space available but minimum is {p}%",
                    avail.space_avail_percent
                ));
            }
            MinFree::Value(n) if avail.space_avail < n => {
                reason.push(format!(
                    "{} ({}) space available but minimum is {} ({})",
                    avail.space_avail.to_formatted_string(&Locale::en),
                    human_bytes(avail.space_avail as f64),
                    n.to_formatted_string(&Locale::en),
                    human_bytes(n as f64),
                ));
            }
            _ => {}
        }
        match self.min_free_inodes {
            MinFree::Percent(p) if avail.inodes_avail_percent < p => {
                reason.push(format!(
                    "{}% inodes available but minimum is {p}%",
                    avail.inodes_avail_percent
                ));
            }
            MinFree::Value(n) if avail.inodes_avail < n => {
                reason.push(format!(
                    "{} inodes available but minimum is {}",
                    avail.space_avail.to_formatted_string(&Locale::en),
                    n.to_formatted_string(&Locale::en),
                ));
            }
            _ => {}
        }

        if reason.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(
                "{} path {} has issue(s): {}",
                self.name,
                self.path.display(),
                reason.join(", ")
            );
        }
    }
}

pub fn is_over_limit() -> bool {
    OVER_LIMIT.load(Ordering::SeqCst)
}

fn copy_paths() -> Vec<MonitoredPath> {
    PATHS.lock().unwrap().clone()
}

fn monitor_thread() {
    let mut bad_monitors = HashSet::new();
    loop {
        let paths = copy_paths();

        for p in paths {
            match p.get_usage() {
                Ok(avail) => match p.check_usage(&avail) {
                    Ok(()) => {
                        if bad_monitors.remove(&p) {
                            tracing::error!("{} path {} has recovered", p.name, p.path.display());
                        }
                    }
                    Err(err) => {
                        if bad_monitors.insert(p.clone()) {
                            tracing::error!("{err:#}");
                        }
                    }
                },
                Err(err) => {
                    tracing::error!("{err:#}");
                }
            }
        }

        OVER_LIMIT.store(!bad_monitors.is_empty(), Ordering::SeqCst);
        std::thread::sleep(Duration::from_secs(5));
    }
}
