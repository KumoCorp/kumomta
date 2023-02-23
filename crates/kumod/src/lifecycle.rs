//! This module helps to manage the life cycle of the process
//! and to shut things down gracefully.
//!
//! See <https://tokio.rs/tokio/topics/shutdown> for more information.
use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc::{Receiver as MPSCReceiver, Sender as MPSCSender};
use tokio::sync::watch::{Receiver as WatchReceiver, Sender as WatchSender};

static ACTIVE: OnceCell<Mutex<Option<Activity>>> = OnceCell::new();
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static STOPPING: OnceCell<ShutdownState> = OnceCell::new();

/// Represents some activity which cannot be ruthlessly interrupted.
/// Obtain an Activity instance via Activity::get(). While any
/// Activity instances are alive in the program, LifeCycle::wait_for_shutdown
/// cannot complete.
#[derive(Clone)]
pub struct Activity {
    _tx: MPSCSender<()>,
}

impl std::fmt::Debug for Activity {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("Activity").finish()
    }
}

impl Activity {
    /// Obtain an Activity instance.
    /// If None is returned then the process is shutting down
    /// and no new activity can be initiated.
    pub fn get_opt() -> Option<Self> {
        Some(ACTIVE.get()?.lock().unwrap().as_ref()?.clone())
    }

    /// Obtain an Activity instance.
    /// Returns Err if the process is shutting down and no new
    /// activity can be initiated
    pub fn get() -> anyhow::Result<Self> {
        Self::get_opt().ok_or_else(|| anyhow::anyhow!("shutting down"))
    }

    /// Returns true if the process is shutting down.
    pub fn is_shutting_down(&self) -> bool {
        SHUTTING_DOWN.load(Ordering::Relaxed)
    }
}

struct ShutdownState {
    tx: WatchSender<()>,
    rx: WatchReceiver<()>,
    request_shutdown_tx: MPSCSender<()>,
}

/// ShutdownSubcription can be used by code that is idling.
/// Select on your timeout and ShutdownSubcription::shutting_down
/// to wake up when either the timeout expires or the process is
/// about to shut down.
pub struct ShutdownSubcription {
    rx: WatchReceiver<()>,
}

impl ShutdownSubcription {
    /// Obtain a shutdown subscription
    pub fn get() -> Self {
        Self {
            rx: STOPPING.get().unwrap().rx.clone(),
        }
    }

    /// Await the shutdown of the process
    pub async fn shutting_down(&mut self) {
        self.rx.changed().await.ok();
    }
}

/// The LifeCycle struct represents the life_cycle of this server process.
/// Creating an instance of it will prepare the global state of the
/// process and allow other code to work with Activity and ShutdownSubcription.
pub struct LifeCycle {
    activity_rx: MPSCReceiver<()>,
    request_shutdown_rx: MPSCReceiver<()>,
}

impl LifeCycle {
    /// Initialize the process life_cycle.
    /// May be called only once; will panic if called multiple times.
    pub fn new() -> Self {
        let (activity_tx, activity_rx) = tokio::sync::mpsc::channel(1);
        ACTIVE
            .set(Mutex::new(Some(Activity { _tx: activity_tx })))
            .map_err(|_| ())
            .unwrap();

        let (request_shutdown_tx, request_shutdown_rx) = tokio::sync::mpsc::channel(1);

        let (tx, rx) = tokio::sync::watch::channel(());
        STOPPING
            .set(ShutdownState {
                tx,
                rx,
                request_shutdown_tx,
            })
            .map_err(|_| ())
            .unwrap();

        Self {
            activity_rx,
            request_shutdown_rx,
        }
    }

    /// Request that we shutdown the process.
    /// This will cause the wait_for_shutdown method on the process
    /// LifeCycle instance to wake up and initiate the shutdown
    /// procedure.
    pub async fn request_shutdown() {
        if let Some(state) = STOPPING.get() {
            state.request_shutdown_tx.send(()).await.ok();
        }
    }

    /// Wait for a shutdown request, then propagate that state
    /// to running tasks, and then wait for those tasks to complete
    /// before returning to the caller.
    pub async fn wait_for_shutdown(&mut self) {
        // Wait for interrupt
        tracing::debug!("Waiting for interrupt");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = self.request_shutdown_rx.recv() => {}
        };
        println!("Shutdown requested, please wait while work is saved");
        // Signal that we are stopping
        tracing::debug!("Signal tasks that we are stopping");
        SHUTTING_DOWN.store(true, Ordering::SeqCst);
        ACTIVE.get().map(|a| a.lock().unwrap().take());
        STOPPING.get().map(|s| s.tx.send(()).ok());
        // Wait for all pending activity to finish
        tracing::debug!("Waiting for tasks to wrap up");
        self.activity_rx.recv().await;
    }
}
