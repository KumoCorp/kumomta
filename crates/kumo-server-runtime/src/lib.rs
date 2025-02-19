use parking_lot::Mutex;
use prometheus::IntGaugeVec;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

pub static RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("localset", |cpus| cpus / 4, &LOCALSET_THREADS).unwrap());
static PARKED_THREADS: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "thread_pool_parked",
        "number of parked(idle) threads in a thread pool",
        &["pool"]
    )
    .unwrap()
});
static NUM_THREADS: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "thread_pool_size",
        "number of threads in a thread pool",
        &["pool"]
    )
    .unwrap()
});

static RUNTIMES: LazyLock<Mutex<HashMap<String, Runtime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn get_named_runtime(name: &str) -> Option<Runtime> {
    RUNTIMES.lock().get(name).cloned()
}

pub static MAIN_RUNTIME: Mutex<Option<tokio::runtime::Handle>> = Mutex::new(None);

pub fn assign_main_runtime(handle: tokio::runtime::Handle) {
    MAIN_RUNTIME.lock().replace(handle);
}

pub fn get_main_runtime() -> tokio::runtime::Handle {
    MAIN_RUNTIME.lock().as_ref().map(|r| r.clone()).unwrap()
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

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        PARKED_THREADS
            .remove_label_values(&[&self.name_prefix])
            .ok();
        NUM_THREADS.remove_label_values(&[&self.name_prefix]).ok();
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
        let env_name = format!("KUMOD_{}_THREADS", name_prefix.to_uppercase());
        let n_threads = match std::env::var(env_name) {
            Ok(n) => n.parse()?,
            Err(_) => {
                let configured = configured_size.load(Ordering::SeqCst);
                if configured == 0 {
                    let cpus = std::thread::available_parallelism()?.get();
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
