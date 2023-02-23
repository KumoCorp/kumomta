//! This module helps to manage the lifetime of the process
//! and to shut things down gracefully.
//!
//! See <https://tokio.rs/tokio/topics/shutdown> for more information.
use once_cell::sync::OnceCell;
use std::sync::Mutex;
use tokio::sync::mpsc::{Receiver as MPSCReceiver, Sender as MPSCSender};
use tokio::sync::watch::{Receiver as WatchReceiver, Sender as WatchSender};

static ACTIVE: OnceCell<Mutex<Option<Activity>>> = OnceCell::new();
static STOPPING: OnceCell<ShutdownState> = OnceCell::new();

/// Represents some activity which cannot be ruthlessly interrupted.
/// Obtain an Activity instance via Activity::get(). While any
/// Activity instances are alive in the program, Lifetime::wait_for_shutdown
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
    pub fn get() -> Option<Self> {
        Some(ACTIVE.get()?.lock().unwrap().as_ref()?.clone())
    }
}

struct ShutdownState {
    tx: WatchSender<()>,
    rx: WatchReceiver<()>,
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

/// The Lifetime struct represents the lifetime of this server process.
/// Creating an instance of it will prepare the global state of the
/// process and allow other code to work with Activity and ShutdownSubcription.
///
pub struct Lifetime {
    activity_rx: MPSCReceiver<()>,
}

impl Lifetime {
    /// Initialize the process lifetime.
    /// May be called only once; will panic if called multiple times.
    pub fn new() -> Self {
        let (activity_tx, activity_rx) = tokio::sync::mpsc::channel(1);
        ACTIVE
            .set(Mutex::new(Some(Activity { _tx: activity_tx })))
            .map_err(|_| ())
            .unwrap();

        let (tx, rx) = tokio::sync::watch::channel(());
        STOPPING
            .set(ShutdownState { tx, rx })
            .map_err(|_| ())
            .unwrap();

        Self { activity_rx }
    }

    /// Wait for a shutdown request, then propagate that state
    /// to running tasks, and then wait for those tasks to complete
    /// before returning to the caller.
    pub async fn wait_for_shutdown(&mut self) {
        // Wait for interrupt
        println!("Waiting for interrupt");
        tokio::signal::ctrl_c().await.ok();
        // Signal that we are stopping
        println!("Signal tasks that we are stopping");
        ACTIVE.get().map(|a| a.lock().unwrap().take());
        STOPPING.get().map(|s| s.tx.send(()).ok());
        // Wait for all pending activity to finish
        println!("Waiting for tasks to wrap up");
        self.activity_rx.recv().await;
    }
}
