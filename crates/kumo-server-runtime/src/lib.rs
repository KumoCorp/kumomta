use anyhow::Context;
use kumo_prometheus::declare_metric;
use kumo_prometheus::prometheus::IntGaugeVec;
use parking_lot::Mutex;
#[cfg(target_os = "linux")]
use std::collections::BTreeMap;
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::fmt::Write;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Once};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::Duration;

pub static RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("localset", |cpus| cpus / 4, &LOCALSET_THREADS).unwrap());

declare_metric! {
/// number of parked (idle) threads in a thread pool
static PARKED_THREADS: IntGaugeVec(
        "thread_pool_parked",
        &["pool"]);
}

declare_metric! {
/// number of threads in a thread pool
static NUM_THREADS: IntGaugeVec(
        "thread_pool_size",
        &["pool"]);
}

declare_metric! {
/// number of tasks waiting in a thread pool's global queue for a worker
/// to pick them up. A sustained non-zero value means the pool's workers
/// cannot keep up with the work scheduled on that pool.
///
/// {{since('dev')}}
static GLOBAL_QUEUE_DEPTH: IntGaugeVec(
        "thread_pool_global_queue_depth",
        &["pool"]);
}

declare_metric! {
/// number of tasks currently alive in a thread pool
///
/// {{since('dev')}}
static ALIVE_TASKS: IntGaugeVec(
        "thread_pool_alive_tasks",
        &["pool"]);
}

declare_metric! {
/// total time in seconds that a thread pool's workers have spent running
/// tasks, summed across its workers. Refreshed once per second by the
/// runtime metrics sampler.
///
/// {{since('dev')}}
static BUSY_SECONDS: CounterVec(
        "thread_pool_busy_seconds",
        &["pool"]);
}

declare_metric! {
/// number of threads in a thread pool's blocking pool, which runs
/// spawn_blocking work and file IO
///
/// {{since('dev')}}
static BLOCKING_THREADS: IntGaugeVec(
        "thread_pool_blocking_threads",
        &["pool"]);
}

declare_metric! {
/// number of idle threads in a thread pool's blocking pool
///
/// {{since('dev')}}
static IDLE_BLOCKING_THREADS: IntGaugeVec(
        "thread_pool_idle_blocking_threads",
        &["pool"]);
}

declare_metric! {
/// number of blocking tasks waiting for a thread in a thread pool's
/// blocking pool
///
/// {{since('dev')}}
static BLOCKING_QUEUE_DEPTH: IntGaugeVec(
        "thread_pool_blocking_queue_depth",
        &["pool"]);
}

/// Refreshes the per-pool gauges for one runtime from tokio's runtime
/// metrics. Call this repeatedly with the same `last_busy` map for a given
/// pool to get correct busy-seconds deltas.
fn sample_runtime_metrics(last_busy: &mut HashMap<String, Duration>, pool: &str, handle: &Handle) {
    let metrics = handle.metrics();
    let set = |gauge: &IntGaugeVec, value: usize| {
        if let Ok(gauge) = gauge.get_metric_with_label_values(&[pool]) {
            gauge.set(value as i64);
        }
    };
    set(&GLOBAL_QUEUE_DEPTH, metrics.global_queue_depth());
    set(&ALIVE_TASKS, metrics.num_alive_tasks());
    set(&BLOCKING_THREADS, metrics.num_blocking_threads());
    set(&IDLE_BLOCKING_THREADS, metrics.num_idle_blocking_threads());
    set(&BLOCKING_QUEUE_DEPTH, metrics.blocking_queue_depth());

    let busy: Duration = (0..metrics.num_workers())
        .map(|worker| metrics.worker_total_busy_duration(worker))
        .sum();
    let previous = last_busy.insert(pool.to_string(), busy).unwrap_or_default();
    if let Ok(counter) = BUSY_SECONDS.get_metric_with_label_values(&[pool]) {
        counter.inc_by(busy.saturating_sub(previous).as_secs_f64());
    }
}

static METRICS_SAMPLER: Once = Once::new();

/// Starts the thread that samples every runtime's metrics once per second.
fn ensure_metrics_sampler() {
    METRICS_SAMPLER.call_once(|| {
        std::thread::Builder::new()
            .name("runtime metrics sampler".to_string())
            .spawn(|| {
                let mut last_busy = HashMap::new();
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    if let Some(main) = try_get_main_runtime() {
                        sample_runtime_metrics(&mut last_busy, "main", &main);
                    }
                    let handles: Vec<(String, Handle)> = RUNTIMES
                        .lock()
                        .iter()
                        .map(|(name, runtime)| (name.clone(), runtime.handle().clone()))
                        .collect();
                    for (name, handle) in handles {
                        sample_runtime_metrics(&mut last_busy, &name, &handle);
                    }
                }
            })
            .expect("create runtime metrics sampler thread");
    });
}

static RUNTIMES: LazyLock<Mutex<HashMap<String, Runtime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn get_named_runtime(name: &str) -> Option<Runtime> {
    RUNTIMES.lock().get(name).cloned()
}

pub static MAIN_RUNTIME: Mutex<Option<tokio::runtime::Handle>> = Mutex::new(None);

pub fn assign_main_runtime(handle: tokio::runtime::Handle) {
    MAIN_RUNTIME.lock().replace(handle);
    ensure_metrics_sampler();
}

pub fn get_main_runtime() -> tokio::runtime::Handle {
    MAIN_RUNTIME.lock().as_ref().map(|r| r.clone()).unwrap()
}

/// Returns the main runtime's handle, or `None` before
/// `assign_main_runtime` has run, which is the case in unit tests.
pub fn try_get_main_runtime() -> Option<tokio::runtime::Handle> {
    MAIN_RUNTIME.lock().clone()
}

/// Spawn a named future onto the main runtime, or onto the current runtime
/// when the main runtime is unassigned (eg: in unit tests).
pub fn spawn_on_main<FUT, N: AsRef<str>>(
    name: N,
    fut: FUT,
) -> std::io::Result<JoinHandle<FUT::Output>>
where
    FUT: Future + Send + 'static,
    FUT::Output: Send,
{
    let builder = tokio::task::Builder::new().name(name.as_ref());
    match try_get_main_runtime() {
        Some(handle) => builder.spawn_on(fut, &handle),
        None => builder.spawn(fut),
    }
}

static LOCALSET_THREADS: AtomicUsize = AtomicUsize::new(0);

pub fn set_localset_threads(n: usize) {
    LOCALSET_THREADS.store(n, Ordering::SeqCst);
}

struct RuntimeInner {
    tokio_runtime: tokio::runtime::Runtime,
    n_threads: usize,
    name_prefix: String,
}

#[cfg(target_os = "linux")]
fn runtimes_by_name() -> BTreeMap<String, Runtime> {
    RUNTIMES
        .lock()
        .iter()
        .map(|(name, rt)| (name.clone(), rt.clone()))
        .collect()
}

#[cfg(not(target_os = "linux"))]
pub async fn dump_all_runtimes(_timeout_duration: Duration) -> String {
    "Runtime state dumping is not supported on this system".into()
}

// NOTE: at the time of writing, calling this once will prevent a
// subsequent graceful shutdown from completing.
//
// You will need to call this multiple times to allow the graceful
// shutdown to "clock through" and finish successfully.
// I do not know what exactly causes that stutter/stickiness.
#[cfg(target_os = "linux")]
pub async fn dump_all_runtimes(timeout_duration: Duration) -> String {
    let runtimes = runtimes_by_name();
    let mut dumps = vec![];

    async fn collect_dump(
        label: &str,
        handle: &tokio::runtime::Handle,
        timeout_duration: Duration,
    ) -> String {
        match tokio::time::timeout(timeout_duration, handle.dump()).await {
            Err(_) => format!("Runtime {label}: Timeout while collecting runtime dump"),
            Ok(dump) => {
                let label = label.to_string();
                match tokio::task::spawn_blocking(move || {
                    let mut output = format!("Runtime: {label}\n");
                    for (i, task) in dump.tasks().iter().enumerate() {
                        let trace = task.trace();
                        writeln!(&mut output, "{label} TASK {i}:\n{trace}").ok();
                    }
                    output
                })
                .await
                .map_err(|err| format!("spawn_blocking: join failed: {err:#}"))
                {
                    Ok(s) | Err(s) => s,
                }
            }
        }
    }

    dumps.push(collect_dump("main", &get_main_runtime(), timeout_duration).await);
    for (label, rt) in runtimes {
        dumps.push(collect_dump(&label, rt.handle(), timeout_duration).await);
    }

    dumps.join("\n\n")
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        let name = self.name_prefix.as_str();
        PARKED_THREADS.remove_label_values(&[name]).ok();
        NUM_THREADS.remove_label_values(&[name]).ok();
        GLOBAL_QUEUE_DEPTH.remove_label_values(&[name]).ok();
        ALIVE_TASKS.remove_label_values(&[name]).ok();
        BUSY_SECONDS.remove_label_values(&[name]).ok();
        BLOCKING_THREADS.remove_label_values(&[name]).ok();
        IDLE_BLOCKING_THREADS.remove_label_values(&[name]).ok();
        BLOCKING_QUEUE_DEPTH.remove_label_values(&[name]).ok();
    }
}

impl Runtime {
    pub fn new<F>(
        name_prefix: &str,
        default_size: F,
        configured_size: &AtomicUsize,
    ) -> anyhow::Result<Self>
    where
        F: FnOnce(usize) -> usize,
    {
        // `main` is the label the sampler assigns to the main runtime. A named
        // pool using that name would record its metrics under the same
        // `pool="main"` label, mixing its numbers into the main runtime's series.
        if name_prefix == "main" {
            anyhow::bail!("thread pool runtime name `main` is reserved for the main runtime");
        }

        let env_name = format!("KUMOD_{}_THREADS", name_prefix.to_uppercase());
        let n_threads = match std::env::var(env_name) {
            Ok(n) => n.parse()?,
            Err(_) => {
                let configured = configured_size.load(Ordering::SeqCst);
                if configured == 0 {
                    let cpus = available_parallelism()?;
                    (default_size)(cpus).max(1)
                } else {
                    configured
                }
            }
        };

        let num_parked = PARKED_THREADS.get_metric_with_label_values(&[name_prefix])?;
        let num_threads = NUM_THREADS.get_metric_with_label_values(&[name_prefix])?;
        num_threads.set(n_threads as i64);

        let next_id = Arc::new(AtomicUsize::new(0));

        let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_io()
            .enable_time()
            .worker_threads(n_threads)
            .event_interval(
                std::env::var("KUMOD_EVENT_INTERVAL")
                    .ok()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(61),
            )
            .max_io_events_per_tick(
                std::env::var("KUMOD_IO_EVENTS_PER_TICK")
                    .ok()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(1024),
            )
            .on_thread_park({
                let num_parked = num_parked.clone();
                move || {
                    kumo_server_memory::purge_thread_cache();
                    num_parked.inc();
                }
            })
            .thread_name_fn({
                let name_prefix = name_prefix.to_string();
                let next_id = next_id.clone();
                move || {
                    let id = next_id.fetch_add(1, Ordering::SeqCst);
                    format!("{name_prefix}-{id}")
                }
            })
            .max_blocking_threads(
                std::env::var(format!(
                    "KUMOD_{}_MAX_BLOCKING_THREADS",
                    name_prefix.to_uppercase()
                ))
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(512),
            )
            .on_thread_unpark({
                let num_parked = num_parked.clone();
                move || {
                    num_parked.dec();
                }
            })
            .build()?;

        let runtime = Self {
            inner: Arc::new(RuntimeInner {
                tokio_runtime,
                n_threads,
                name_prefix: name_prefix.to_string(),
            }),
        };

        let mut runtimes = RUNTIMES.lock();
        if runtimes.contains_key(name_prefix) {
            anyhow::bail!("thread pool runtime with name `{name_prefix}` already exists!");
        }

        runtimes.insert(name_prefix.to_string(), runtime.clone());
        drop(runtimes);

        ensure_metrics_sampler();
        Ok(runtime)
    }

    pub fn handle(&self) -> &tokio::runtime::Handle {
        self.inner.tokio_runtime.handle()
    }

    pub fn get_num_threads(&self) -> usize {
        self.inner.n_threads
    }

    /// Spawn a future into this runtime
    pub fn spawn<FUT, N: AsRef<str>>(
        &self,
        name: N,
        fut: FUT,
    ) -> std::io::Result<JoinHandle<FUT::Output>>
    where
        FUT: Future + Send + 'static,
        FUT::Output: Send,
    {
        tokio::task::Builder::new()
            .name(name.as_ref())
            .spawn_on(fut, self.handle())
    }
}

/// Schedule func to run in the main runtime pool,
/// which is named "localset" for legacy reasons.
pub fn rt_spawn<FUT, N: AsRef<str>>(name: N, fut: FUT) -> std::io::Result<JoinHandle<FUT::Output>>
where
    FUT: Future + Send + 'static,
    FUT::Output: Send,
{
    tokio::task::Builder::new()
        .name(name.as_ref())
        .spawn_on(fut, RUNTIME.handle())
}

/// Spawn a future as a task with a name.
/// The task is spawned into the current tokio runtime.
pub fn spawn<FUT, N: AsRef<str>>(name: N, fut: FUT) -> std::io::Result<JoinHandle<FUT::Output>>
where
    FUT: Future + Send + 'static,
    FUT::Output: Send,
{
    tokio::task::Builder::new().name(name.as_ref()).spawn(fut)
}

/// Run a blocking function in the worker thread pool associated
/// with the current tokio runtime.
pub fn spawn_blocking<F, N, R>(name: N, func: F) -> std::io::Result<JoinHandle<R>>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
    N: AsRef<str>,
{
    tokio::task::Builder::new()
        .name(name.as_ref())
        .spawn_blocking(func)
}

/// Run a blocking function in the worker thread pool associated
/// with the provided tokio runtime.
pub fn spawn_blocking_on<F, N, R>(
    name: N,
    func: F,
    runtime: &Handle,
) -> std::io::Result<JoinHandle<R>>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
    N: AsRef<str>,
{
    tokio::task::Builder::new()
        .name(name.as_ref())
        .spawn_blocking_on(func, runtime)
}

pub fn available_parallelism() -> anyhow::Result<usize> {
    match std::env::var("KUMO_AVAILABLE_PARALLELISM") {
        Ok(n) => n
            .parse()
            .context("failed to parse KUMO_AVAILABLE_PARALLELISM as a number"),
        Err(_) => Ok(std::thread::available_parallelism()
            .context("failed to get available_parallelism")?
            .get()),
    }
}

/// Decide how long to wait, if at all, before retrying after an error from
/// `accept()`.
///
/// Returns `None` for connection-level errors (the peer went away between the
/// kernel queuing the connection and our accepting it): retry these
/// immediately. Returns a pause for everything else, such as file-descriptor
/// exhaustion (EMFILE/ENFILE): wait out the returned duration before
/// retrying these, since they persist across calls until some other
/// connection closes or a resource frees up.
pub fn accept_error_pause(err: &std::io::Error) -> Option<Duration> {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionAborted
        | ErrorKind::ConnectionReset => None,
        _ => Some(Duration::from_secs(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_error_pause_backs_off_on_fd_exhaustion() {
        // EMFILE maps to an uncategorized ErrorKind on most platforms. The
        // classifier must still pause rather than spin.
        let emfile = std::io::Error::from_raw_os_error(libc::EMFILE);
        assert_eq!(accept_error_pause(&emfile), Some(Duration::from_secs(1)));
    }
}
