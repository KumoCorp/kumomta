//! This module contains logic to reason about memory usage,
//! implement memory limits, and some helpers to attempt to
//! release cached memory back to the system when memory
//! is low.

#[cfg(target_os = "linux")]
use cgroups_rs::cgroup::{get_cgroups_relative_paths, Cgroup, UNIFIED_MOUNTPOINT};
#[cfg(target_os = "linux")]
use cgroups_rs::hierarchies::{V1, V2};
#[cfg(target_os = "linux")]
use cgroups_rs::memory::MemController;
#[cfg(target_os = "linux")]
use cgroups_rs::{Hierarchy, MaxValue};
use nix::sys::resource::{rlim_t, RLIM_INFINITY};
use nix::unistd::{sysconf, SysconfVar};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tikv_jemallocator::Jemalloc;
use tokio::sync::watch::Receiver;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

lazy_static::lazy_static! {
    static ref OVER_LIMIT_COUNT: metrics::Counter = {
        metrics::describe_counter!(
            "memory_over_limit_count",
            "how many times the soft memory limit was exceeded");
        metrics::register_counter!("memory_over_limit_count")
    };
    static ref MEM_USAGE: metrics::Gauge = {
        metrics::describe_gauge!(
            "memory_usage", "number of bytes of used memory");
        metrics::register_gauge!("memory_usage")
    };
    static ref MEM_LIMIT: metrics::Gauge = {
        metrics::describe_gauge!(
            "memory_limit", "soft memory limit measured in bytes");
        metrics::register_gauge!("memory_limit")
    };
    static ref SUBSCRIBER: Mutex<Option<Receiver<()>>> = Mutex::new(None);
}

static OVER_LIMIT: AtomicBool = AtomicBool::new(false);
static LOW_MEM: AtomicBool = AtomicBool::new(false);
static HEAD_ROOM: AtomicUsize = AtomicUsize::new(0);

/// Represents the current memory usage of this process
#[derive(Debug, Clone, Copy)]
pub struct MemoryUsage {
    pub bytes: u64,
}

impl std::fmt::Display for MemoryUsage {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}", human(self.bytes))
    }
}

impl MemoryUsage {
    pub fn get() -> anyhow::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            if let Ok(v2) = Self::get_cgroup(true) {
                return Ok(v2);
            }
            if let Ok(v1) = Self::get_cgroup(false) {
                return Ok(v1);
            }
        }
        Self::get_linux_statm()
    }

    #[cfg(target_os = "linux")]
    fn get_cgroup(v2: bool) -> anyhow::Result<Self> {
        let cgroup = get_my_cgroup(v2)?;
        let mem: &MemController = cgroup
            .controller_of()
            .ok_or_else(|| anyhow::anyhow!("no memory controller?"))?;
        let stat = mem.memory_stat();
        Ok(Self {
            bytes: stat.usage_in_bytes,
        })
    }

    pub fn get_linux_statm() -> anyhow::Result<Self> {
        let data = std::fs::read_to_string("/proc/self/statm")?;
        let fields: Vec<&str> = data.split(' ').collect();
        let rss: u64 = fields[1].parse()?;
        Ok(Self {
            bytes: rss * sysconf(SysconfVar::PAGE_SIZE)?.unwrap_or(4 * 1024) as u64,
        })
    }
}

fn human(n: u64) -> String {
    humansize::format_size(n, humansize::DECIMAL)
}

/// Represents a constraint on memory usage
#[derive(Debug, Clone, Copy)]
pub struct MemoryLimits {
    pub soft_limit: Option<u64>,
    pub hard_limit: Option<u64>,
}

impl std::fmt::Display for MemoryLimits {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let soft = self.soft_limit.map(human);
        let hard = self.hard_limit.map(human);
        write!(fmt, "soft={soft:?}, hard={hard:?}")
    }
}

impl MemoryLimits {
    pub fn min(self, other: Self) -> Self {
        Self {
            soft_limit: min_opt_limit(self.soft_limit, other.soft_limit),
            hard_limit: min_opt_limit(self.hard_limit, other.hard_limit),
        }
    }

    pub fn is_unlimited(&self) -> bool {
        self.soft_limit.is_none() && self.hard_limit.is_none()
    }
}

fn rlim_to_opt(rlim: rlim_t) -> Option<u64> {
    if rlim == RLIM_INFINITY {
        None
    } else {
        Some(rlim)
    }
}

#[cfg(target_os = "linux")]
fn max_value_to_opt(value: Option<MaxValue>) -> anyhow::Result<Option<u64>> {
    Ok(match value {
        None | Some(MaxValue::Max) => None,
        Some(MaxValue::Value(n)) if n >= 0 => Some(n as u64),
        Some(MaxValue::Value(n)) => anyhow::bail!("unexpected negative limit {n}"),
    })
}

fn min_opt_limit(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

impl MemoryLimits {
    pub fn get_rlimits() -> anyhow::Result<Self> {
        #[cfg(not(target_os = "macos"))]
        let (rss_soft, rss_hard) =
            nix::sys::resource::getrlimit(nix::sys::resource::Resource::RLIMIT_RSS)?;
        #[cfg(target_os = "macos")]
        let (rss_soft, rss_hard) = (RLIM_INFINITY, RLIM_INFINITY);

        let soft_limit = rlim_to_opt(rss_soft);
        let hard_limit = rlim_to_opt(rss_hard);

        Ok(Self {
            soft_limit,
            hard_limit,
        })
    }

    #[cfg(target_os = "linux")]
    fn get_any_cgroup() -> anyhow::Result<Self> {
        if let Ok(cg) = Self::get_cgroup(true) {
            return Ok(cg);
        }
        Self::get_cgroup(false)
    }

    #[cfg(target_os = "linux")]
    pub fn get_cgroup(v2: bool) -> anyhow::Result<Self> {
        let cgroup = get_my_cgroup(v2)?;
        let mem: &MemController = cgroup
            .controller_of()
            .ok_or_else(|| anyhow::anyhow!("no memory controller?"))?;

        let limits = mem.get_mem()?;
        Ok(Self {
            soft_limit: max_value_to_opt(limits.high)?,
            hard_limit: max_value_to_opt(limits.max)?,
        })
    }
}

/// Returns the amount of physical memory available to the system.
/// This is linux specific.
#[cfg(target_os = "linux")]
fn get_physical_memory() -> anyhow::Result<u64> {
    let data = std::fs::read_to_string("/proc/meminfo")?;
    for line in data.lines() {
        if line.starts_with("MemTotal:") {
            let mut iter = line.rsplit(' ');
            let unit = iter
                .next()
                .ok_or_else(|| anyhow::anyhow!("expected unit"))?;
            if unit != "kB" {
                anyhow::bail!("unsupported /proc/meminfo unit {unit}");
            }
            let value = iter
                .next()
                .ok_or_else(|| anyhow::anyhow!("expected value"))?;
            let value: u64 = value.parse()?;

            return Ok(value * 1024);
        }
    }
    anyhow::bail!("MemTotal not found in /proc/meminfo");
}

/// Retrieves the current usage and limits.
/// This is a bit of a murky area as, on Linux, the cgroup reported usage
/// appears to be nonsensical when no limits are configured.
/// So we first obtain the limits from cgroups, and if they are set,
/// we return the usage from cgroups along with it,
/// otherwise we get the ulimit limits and look at the more general
/// usage numbers to go with it.
///
/// If no limits are explicitly configured, we'll assume a hard limit
/// equal to the physical ram on the system, and a soft limit of 75%
/// of whatever we've determined the hard limit to be.
#[cfg(target_os = "linux")]
pub fn get_usage_and_limit() -> anyhow::Result<(MemoryUsage, MemoryLimits)> {
    let mut limit = MemoryLimits::get_rlimits()?;
    let mut usage = MemoryUsage::get_linux_statm()?;

    if let Ok(cg_lim) = MemoryLimits::get_any_cgroup() {
        if !cg_lim.is_unlimited() {
            limit = limit.min(cg_lim);
            usage = MemoryUsage::get()?;
        }
    }

    let phys = get_physical_memory()?;
    if limit.hard_limit.is_none() {
        limit.hard_limit.replace(phys);
    }
    if limit.soft_limit.is_none() {
        limit.soft_limit = limit.hard_limit.map(|lim| lim * 3 / 4);
    }

    Ok((usage, limit))
}

#[cfg(not(target_os = "linux"))]
pub fn get_usage_and_limit() -> anyhow::Result<(MemoryUsage, MemoryLimits)> {
    Ok((
        MemoryUsage { bytes: 0 },
        MemoryLimits {
            soft_limit: None,
            hard_limit: None,
        },
    ))
}

/// To be called when a thread goes idle; it will flush cached
/// memory out of the thread local cache to be returned/reused
/// elsewhere in the system
pub fn purge_thread_cache() {
    unsafe {
        tikv_jemalloc_sys::mallctl(
            b"thread.tcache.flush\0".as_ptr() as *const _,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        );
    }
}

/// To be called when used memory is high: will aggressively
/// flush and release cached memory
fn purge_all_arenas() {
    unsafe {
        // 4096 is MALLCTL_ARENAS_ALL, which is a magic value
        // that instructs jemalloc to purge all arenas
        tikv_jemalloc_sys::mallctl(
            b"arena.4096.purge\0".as_ptr() as *const _,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        );
    }
}

/// If `MALLOC_CONF='prof:true,prof_prefix:jeprof.out'` is set in the
/// environment, calling this will generate a heap profile in the
/// current directory
fn dump_heap_profile() {
    unsafe {
        tikv_jemalloc_sys::mallctl(
            b"prof.dump\0".as_ptr() as *const _,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        );
    }
}

/// The memory thread continuously examines memory usage and limits
/// and maintains global counters to track the memory state
fn memory_thread() {
    let mut is_ok = true;

    let (tx, rx) = tokio::sync::watch::channel(());
    SUBSCRIBER.lock().unwrap().replace(rx);

    loop {
        match get_usage_and_limit() {
            Ok((
                MemoryUsage { bytes: usage },
                MemoryLimits {
                    soft_limit: Some(limit),
                    hard_limit: _,
                },
            )) => {
                let was_ok = is_ok;
                is_ok = usage < limit;
                OVER_LIMIT.store(is_ok, Ordering::SeqCst);
                HEAD_ROOM.store(limit.saturating_sub(usage) as usize, Ordering::SeqCst);
                MEM_USAGE.set(usage as f64);
                MEM_LIMIT.set(limit as f64);

                let low_thresh = limit * 8 / 10;
                LOW_MEM.store(usage > low_thresh, Ordering::SeqCst);

                if !is_ok && was_ok {
                    // Transition from OK -> !OK
                    dump_heap_profile();
                    OVER_LIMIT_COUNT.increment(1);
                    tracing::error!(
                        "memory usage {} exceeds limit {}",
                        human(usage),
                        human(limit)
                    );
                    tx.send(()).ok();
                    purge_all_arenas();
                } else if !was_ok && is_ok {
                    // Transition from !OK -> OK
                    dump_heap_profile();
                    tracing::error!(
                        "memory usage {} is back within limit {}",
                        human(usage),
                        human(limit)
                    );
                    tx.send(()).ok();
                } else {
                    if !is_ok {
                        purge_all_arenas();
                    }
                    tracing::debug!("memory usage {}, limit {}", human(usage), human(limit));
                }
            }
            Ok((
                MemoryUsage { bytes: 0 },
                MemoryLimits {
                    soft_limit: None,
                    hard_limit: None,
                },
            )) => {
                // We don't know anything about the memory usage on this
                // system, just pretend everything is fine
                HEAD_ROOM.store(1024, Ordering::SeqCst);
            }
            Ok(_) => {}
            Err(err) => tracing::error!("unable to query memory info: {err:#}"),
        }

        std::thread::sleep(Duration::from_secs(3));
    }
}

/// Returns the amount of headroom; the number of bytes that can
/// be allocated before we hit the soft limit
pub fn get_headroom() -> usize {
    HEAD_ROOM.load(Ordering::SeqCst)
}

/// Returns true when we are within 10% if the soft limit
pub fn low_memory() -> bool {
    LOW_MEM.load(Ordering::SeqCst)
}

/// Returns a receiver that will notify when memory status
/// changes from OK -> !OK or vice versa.
pub fn subscribe_to_memory_status_changes() -> Receiver<()> {
    SUBSCRIBER.lock().unwrap().clone().unwrap()
}

/// Initialize the memory thread to monitor memory usage/limits
pub fn setup_memory_limit() -> anyhow::Result<()> {
    let (usage, limit) = get_usage_and_limit()?;
    tracing::debug!("usage: {usage:?}");
    tracing::info!("using limits: {limit}");

    std::thread::Builder::new()
        .name("memory-monitor".to_string())
        .spawn(memory_thread)?;

    Ok(())
}

/// Returns a Cgroup for the current process.
/// Can return either a v2 or a v1 cgroup.
#[cfg(target_os = "linux")]
fn get_my_cgroup(v2: bool) -> anyhow::Result<Cgroup> {
    let paths = get_cgroups_relative_paths()?;
    let h: Box<dyn Hierarchy> = if v2 {
        Box::new(V2::new())
    } else {
        Box::new(V1::new())
    };

    let path = paths
        .get("")
        .ok_or_else(|| anyhow::anyhow!("couldn't resolve path"))?;

    let cgroup = Cgroup::load(h, format!("{}/{}", UNIFIED_MOUNTPOINT, path));
    Ok(cgroup)
}
