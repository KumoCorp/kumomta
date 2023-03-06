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
use std::future::Future;
use tokio::task::{JoinHandle, LocalSet};

lazy_static::lazy_static! {
    static ref RUNTIME: Runtime = Runtime::new().unwrap();
}

enum Command {
    Run(Box<dyn FnOnce() + Send>),
}

pub struct Runtime {
    jobs: Sender<Command>,
}

impl Runtime {
    pub fn new() -> anyhow::Result<Self> {
        let n_threads = std::thread::available_parallelism()?;
        let (tx, rx) = unbounded::<Command>();

        for n in 0..n_threads.into() {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("localset-{n}"))
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_io()
                        .enable_time()
                        .build()
                        .unwrap();
                    let local_set = LocalSet::new();

                    local_set.block_on(&runtime, async move {
                        tracing::trace!("localset-{n} started up!");
                        while let Ok(cmd) = rx.recv().await {
                            match cmd {
                                Command::Run(func) => (func)(),
                            }
                        }
                    });
                })?;
        }

        Ok(Self { jobs: tx })
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
pub fn rt_spawn<F: (FnOnce() -> anyhow::Result<FUT>) + Send + 'static, FUT>(
    name: String,
    func: F,
) -> anyhow::Result<JoinHandle<FUT::Output>>
where
    FUT: Future + 'static,
    FUT::Output: Send,
{
    let (tx, rx) = bounded::<anyhow::Result<JoinHandle<FUT::Output>>>(1);
    RUNTIME
        .jobs
        .send_blocking(Command::Run(Box::new(move || match (func)() {
            Ok(future) => {
                tx.send_blocking(
                    tokio::task::Builder::new()
                        .name(&name)
                        .spawn_local(future)
                        .map_err(|e| e.into()),
                )
                .ok();
            }
            Err(err) => {
                tx.send_blocking(Err(err)).ok();
            }
        })))
        .map_err(|err| anyhow::anyhow!("failed to send func to runtime thread: {err:#}"))?;
    rx.recv_blocking()?
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
