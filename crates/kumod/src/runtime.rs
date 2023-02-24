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
use async_channel::{unbounded, Sender};
use tokio::task::LocalSet;

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

    /// Schedule a future on the pool.
    /// Will return once the future is scheduled.
    /// Does not wait for the future to complete.
    pub async fn run<F: FnOnce() + Send + 'static>(func: F) -> anyhow::Result<()> {
        RUNTIME
            .jobs
            .send(Command::Run(Box::new(func)))
            .await
            .map_err(|err| anyhow::anyhow!("failed to send func to runtime thread: {err:#}"))?;
        Ok(())
    }
}
