//! This is a little helper for routing small units of work to
//! a thread pool that runs a LocalSet + single threaded tokio
//! runtime.
//!
//! The rationale for this is that mlua doesn't currently provide
//! async call implementations that are Send, so it is not possible
//! to use them within the usual Send Future that is required by
//! the normal tokio::spawn function.
//!
//! We use Runtime::run() to spawn a little closure whose purpose
//! is to use tokio::task::spawn_local to spawn the local future.
//! For example, when accepting new connections, we use this to
//! spawn the server processing future.
use prometheus::IntGaugeVec;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::runtime::Handle;
use tokio::task::{JoinHandle, LocalSet};

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

pub static MAIN_RUNTIME: std::sync::Mutex<Option<tokio::runtime::Handle>> =
    std::sync::Mutex::new(None);

pub fn assign_main_runtime(handle: tokio::runtime::Handle) {
    MAIN_RUNTIME.lock().unwrap().replace(handle);
}

pub fn get_main_runtime() -> tokio::runtime::Handle {
    MAIN_RUNTIME
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| r.clone())
        .unwrap()
}

static LOCALSET_THREADS: AtomicUsize = AtomicUsize::new(0);

pub fn set_localset_threads(n: usize) {
    LOCALSET_THREADS.store(n, Ordering::SeqCst);
}

pub struct Runtime {
    tokio_runtime: tokio::runtime::Runtime,
    n_threads: usize,
    name_prefix: &'static str,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        PARKED_THREADS.remove_label_values(&[self.name_prefix]).ok();
        NUM_THREADS.remove_label_values(&[self.name_prefix]).ok();
    }
}

pub fn spawn_simple_worker_pool<SIZE, FUNC, FUT>(
    name_prefix: &'static str,
    default_size: SIZE,
    configured_size: &AtomicUsize,
    func_factory: FUNC,
) -> anyhow::Result<usize>
where
    SIZE: FnOnce(usize) -> usize,
    FUNC: (Fn() -> FUT) + Send + Sync + 'static,
    FUT: Future + 'static,
    FUT::Output: Send,
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

    let func_factory = Arc::new(func_factory);

    for n in 0..n_threads.into() {
        let num_parked = num_parked.clone();
        let func_factory = func_factory.clone();
        std::thread::Builder::new()
            .name(format!("{name_prefix}-{n}"))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
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
                    .thread_name(format!("{name_prefix}-blocking"))
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
                    .build()
                    .unwrap();
                let local_set = LocalSet::new();

                local_set.block_on(&runtime, async move {
                    if n == 0 {
                        tracing::info!("{name_prefix} pool starting with {n_threads} threads");
                    }
                    tracing::trace!("{name_prefix}-{n} started up!");
                    let func = (func_factory)();
                    (func).await
                });
            })?;
    }
    Ok(n_threads)
}

impl Runtime {
    pub fn new<F>(
        name_prefix: &'static str,
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

        let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_io()
            .enable_time()
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
                move || {
                    static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
                    let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
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

        Ok(Self {
            tokio_runtime,
            n_threads,
            name_prefix,
        })
    }

    pub fn handle(&self) -> &tokio::runtime::Handle {
        self.tokio_runtime.handle()
    }

    pub fn get_num_threads(&self) -> usize {
        self.n_threads
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
