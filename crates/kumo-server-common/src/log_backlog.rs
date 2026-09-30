//! Expose, as metrics, how far each out-of-process log consumer trails the
//! newest data in the directories to which this node writes logs.
//!
//! A directory holding more than one log stream, such as several writers with
//! distinct suffixes sharing one directory, is unsupported: the scan counts
//! every segment matching the pattern, regardless of which writer produced it,
//! so a consumer reading only one of the streams is reported against the
//! combined backlog of all of them.

pub use kumo_jsonl::WriterLocation;
use kumo_prometheus::declare_metric;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, Once};
use std::time::Duration;

/// How often the monitor rescans every registered location, in milliseconds.
static SCAN_INTERVAL_MS: AtomicU64 = AtomicU64::new(30_000);

/// Set how often the monitor rescans registered locations, effective on its
/// next cycle.
pub fn set_scan_interval(interval: Duration) {
    SCAN_INTERVAL_MS.store(interval.as_millis() as u64, Ordering::Relaxed);
}

fn scan_interval() -> Duration {
    Duration::from_millis(SCAN_INTERVAL_MS.load(Ordering::Relaxed))
}

static LOCATIONS: LazyLock<Mutex<HashSet<WriterLocation>>> = LazyLock::new(Default::default);
static MONITOR: Once = Once::new();

declare_metric! {
/// The number of whole log segments not yet processed by a consumer.
///
/// Labelled by:
/// * `log_dir` - the log directory being scanned
/// * `checkpoint_name` - the name set by the tailer of the consumer for its
///   checkpoint.
///
/// A sample value is
/// `log_consumer_segments_behind{log_dir="/var/log/kumomta",checkpoint_name="cp"} 3`.
static SEGMENTS_BEHIND: IntGaugeVec(
        "log_consumer_segments_behind",
        &["log_dir", "checkpoint_name"]
    );
}

declare_metric! {
/// The compressed size, in bytes, of whole log segments awaiting processing by
/// a consumer.
///
/// This counts whole unread segments only, not the remainder of the segment
/// that the consumer is partway through processing.
///
/// Labelled by:
/// * `log_dir` - the log directory being scanned
/// * `checkpoint_name` - the name set by the tailer of the consumer for its
///   checkpoint.
static BYTES_BEHIND: IntGaugeVec(
        "log_consumer_bytes_behind",
        &["log_dir", "checkpoint_name"]
    );
}

declare_metric! {
/// Age in seconds of the oldest log segment not yet started by a consumer,
/// counted from the modification time of that segment.
///
/// Such a segment does not exist once the consumer has reached the newest log
/// segment. The gauge reports 0 in that case.
///
/// Labelled by:
///
/// * `log_dir` - the log directory being scanned
/// * `checkpoint_name` - the name set by the tailer of the consumer for its
///   checkpoint.
static LAG_SECONDS: IntGaugeVec(
        "log_consumer_lag_seconds",
        &["log_dir", "checkpoint_name"]
    );
}

/// Add `location` to the monitored set and start the monitor if it is not
/// already running. Registering the same location more than once, including
/// when a configuration reload re-registers the same directory, has the same
/// effect as registering it once. A location is monitored for the lifetime of
/// the process. A location cannot be deregistered.
pub fn register_location(location: WriterLocation) {
    LOCATIONS
        .lock()
        .expect("log backlog locations mutex poisoned")
        .insert(location);

    start_monitor();
}

/// Start the background monitor thread if it is not already running.
pub fn start_monitor() {
    MONITOR.call_once(|| {
        // Allow tests to override the scan interval. Invalid values are
        // silently ignored.
        if let Some(interval) = std::env::var("KUMOD_LOG_BACKLOG_SCAN_INTERVAL")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .and_then(|v| Duration::try_from_secs_f64(v).ok())
        {
            set_scan_interval(interval);
        }
        std::thread::Builder::new()
            .name("log-backlog-monitor".to_string())
            .spawn(monitor_thread)
            .expect("failed to spawn log-backlog-monitor thread");
    });
}

/// Returns the current set of locations that require backlog monitoring.
fn monitored_locations() -> HashSet<WriterLocation> {
    let mut locations: HashSet<WriterLocation> = LOCATIONS
        .lock()
        .expect("log backlog locations mutex poisoned")
        .clone();
    locations.extend(kumo_jsonl::active_writer_locations());
    locations
}

/// Publish the backlog gauges for `location` and return the set of checkpoint
/// names reported by it. Returns `None` on a scan error.
fn scan_location(location: &WriterLocation) -> Option<HashSet<String>> {
    let reports = match kumo_jsonl::scan_backlog(&location.directory, &location.pattern) {
        Ok(reports) => reports,
        Err(err) => {
            tracing::error!("scanning log backlog in {}: {err:#}", location.directory);
            return None;
        }
    };

    let dir = location.directory.as_str();
    let mut checkpoint_names = HashSet::new();
    for report in reports {
        let labels = [dir, report.checkpoint_name.as_str()];
        if let Ok(gauge) = SEGMENTS_BEHIND.get_metric_with_label_values(&labels) {
            gauge.set(report.segments_behind as i64);
        }
        if let Ok(gauge) = BYTES_BEHIND.get_metric_with_label_values(&labels) {
            gauge.set(report.bytes_behind as i64);
        }
        if let Ok(gauge) = LAG_SECONDS.get_metric_with_label_values(&labels) {
            gauge.set(report.lag.map(|d| d.as_secs()).unwrap_or(0) as i64);
        }
        checkpoint_names.insert(report.checkpoint_name);
    }
    Some(checkpoint_names)
}

/// Remove the gauges for a consumer whose checkpoint has disappeared since the
/// previous scan to avoid leaving a stale reading behind.
fn forget_checkpoint(dir: &str, checkpoint_name: &str) {
    let labels = [dir, checkpoint_name];
    let _ = SEGMENTS_BEHIND.remove_label_values(&labels);
    let _ = BYTES_BEHIND.remove_label_values(&labels);
    let _ = LAG_SECONDS.remove_label_values(&labels);
}

fn monitor_thread() {
    let mut previous: HashMap<WriterLocation, HashSet<String>> = HashMap::new();
    loop {
        let locations = monitored_locations();

        previous.retain(|location, names| {
            if locations.contains(location) {
                return true;
            }
            // Clear gauges for inactive locations because Prometheus retains
            // their last values otherwise.
            for name in names.iter() {
                forget_checkpoint(location.directory.as_str(), name);
            }
            false
        });

        for location in &locations {
            let Some(current) = scan_location(location) else {
                continue;
            };
            if let Some(prior) = previous.get(location) {
                for gone in prior.difference(&current) {
                    forget_checkpoint(location.directory.as_str(), gone);
                }
            }
            previous.insert(location.clone(), current);
        }
        std::thread::sleep(scan_interval());
    }
}
