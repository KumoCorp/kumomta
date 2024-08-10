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
use async_channel::{bounded, unbounded, Sender};
use prometheus::IntGaugeVec;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::runtime::Handle;
use tokio::task::{JoinHandle, LocalSet};

lazy_static::lazy_static! {
    pub static ref RUNTIME: Runtime = Runtime::new(
        "localset", |cpus| cpus/4, &LOCALSET_THREADS).unwrap();

    static ref PARKED_THREADS: IntGaugeVec = {
        prometheus::register_int_gauge_vec!(
            "thread_pool_parked",
            "number of parked(idle) threads in a thread pool",
            &["pool"]).unwrap()
    };
    static ref NUM_THREADS: IntGaugeVec = {
        prometheus::register_int_gauge_vec!(
            "thread_pool_size",
            "number of threads in a thread pool",
            &["pool"]).unwrap()
    };
}

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

enum Command {
    Run(Box<dyn FnOnce() + Send>),
}

pub struct Runtime {
    jobs: Sender<Command>,
    n_threads: usize,
    name_prefix: &'static str,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        PARKED_THREADS.remove_label_values(&[self.name_prefix]).ok();
        NUM_THREADS.remove_label_values(&[self.name_prefix]).ok();
    }
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
        let (tx, rx) = unbounded::<Command>();

        let num_parked = PARKED_THREADS.get_metric_with_label_values(&[name_prefix])?;
        let num_threads = NUM_THREADS.get_metric_with_label_values(&[name_prefix])?;
        num_threads.set(n_threads as i64);

        for n in 0..n_threads.into() {
            let rx = rx.clone();
            let num_parked = num_parked.clone();
            std::thread::Builder::new()
                .name(format!("{name_prefix}-{n}"))
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_io()
                        .enable_time()
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
                        while let Ok(cmd) = rx.recv().await {
                            match cmd {
                                Command::Run(func) => (func)(),
                            }
                        }
                    });
                })?;
        }

        Ok(Self {
            jobs: tx,
            n_threads,
            name_prefix,
        })
    }

    pub fn get_num_threads(&self) -> usize {
        self.n_threads
    }

    /// Schedule func to run in the runtime pool.
    /// func must return a future; that future will be spawned into the thread-local
    /// executor.
    /// This function will return the result of the spawn attempt, but will not
    /// wait for the future it spawns to complete.
    ///
    /// This function is useful for getting into a localset environment where
    /// !Send futures can be scheduled when you are not already in such an environment.
    ///
    /// If you are already in a !Send future, then using spawn_local below has
    /// less overhead.
    pub async fn spawn<F: (FnOnce() -> anyhow::Result<FUT>) + Send + 'static, FUT>(
        &self,
        name: String,
        func: F,
    ) -> anyhow::Result<JoinHandle<FUT::Output>>
    where
        FUT: Future + 'static,
        FUT::Output: Send,
    {
        let (tx, rx) = bounded::<anyhow::Result<JoinHandle<FUT::Output>>>(1);
        self.jobs
            .try_send(Command::Run(Box::new(move || match (func)() {
                Ok(future) => {
                    tx.try_send(
                        tokio::task::Builder::new()
                            .name(&name)
                            .spawn_local(future)
                            .map_err(|e| e.into()),
                    )
                    .ok();
                }
                Err(err) => {
                    tx.try_send(Err(err)).ok();
                }
            })))
            .map_err(|err| anyhow::anyhow!("failed to send func to runtime thread: {err:#}"))?;
        rx.recv().await?
    }

    pub fn spawn_non_blocking<F: (FnOnce() -> anyhow::Result<FUT>) + Send + 'static, FUT>(
        &self,
        name: String,
        func: F,
    ) -> anyhow::Result<()>
    where
        FUT: Future + 'static,
    {
        self.jobs
            .try_send(Command::Run(Box::new(move || match (func)() {
                Ok(future) => {
                    if let Err(err) = tokio::task::Builder::new().name(&name).spawn_local(future) {
                        tracing::error!("rt_spawn_non_blocking: error: {err:#}");
                    }
                }
                Err(err) => {
                    tracing::error!("rt_spawn_non_blocking: error: {err:#}");
                }
            })))
            .map_err(|err| anyhow::anyhow!("failed to send func to runtime thread: {err:#}"))
    }
}

/// Schedule func to run in the runtime pool.
/// func must return a future; that future will be spawned into the thread-local
/// executor.
/// This function will return the result of the spawn attempt, but will not
/// wait for the future it spawns to complete.
///
/// This function is useful for getting into a localset environment where
/// !Send futures can be scheduled when you are not already in such an environment.
///
/// If you are already in a !Send future, then using spawn_local below has
/// less overhead.
pub async fn rt_spawn<F: (FnOnce() -> anyhow::Result<FUT>) + Send + 'static, FUT>(
    name: String,
    func: F,
) -> anyhow::Result<JoinHandle<FUT::Output>>
where
    FUT: Future + 'static,
    FUT::Output: Send,
{
    RUNTIME.spawn(name, func).await
}

pub fn rt_spawn_non_blocking<F: (FnOnce() -> anyhow::Result<FUT>) + Send + 'static, FUT>(
    name: String,
    func: F,
) -> anyhow::Result<()>
where
    FUT: Future + 'static,
{
    RUNTIME.spawn_non_blocking(name, func)
}

/// Spawn a future as a task with a name.
pub fn spawn<FUT, N: AsRef<str>>(name: N, fut: FUT) -> std::io::Result<JoinHandle<FUT::Output>>
where
    FUT: Future + Send + 'static,
    FUT::Output: Send,
{
    tokio::task::Builder::new().name(name.as_ref()).spawn(fut)
}

/// Spawn a local future as a task with a name.
pub fn spawn_local<FUT, N: AsRef<str>>(
    name: N,
    fut: FUT,
) -> std::io::Result<JoinHandle<FUT::Output>>
where
    FUT: Future + 'static,
    FUT::Output: Send,
{
    tokio::task::Builder::new()
        .name(name.as_ref())
        .spawn_local(fut)
}

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
