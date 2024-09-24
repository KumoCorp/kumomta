//! This module helps to manage the life cycle of the process
//! and to shut things down gracefully.
//!
//! See <https://tokio.rs/tokio/topics/shutdown> for more information.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use tokio::signal::unix::SignalKind;
use tokio::sync::mpsc::{Receiver as MPSCReceiver, Sender as MPSCSender};
use tokio::sync::watch::{Receiver as WatchReceiver, Sender as WatchSender};
use uuid::Uuid;

static ACTIVE: OnceLock<Mutex<Option<Activity>>> = OnceLock::new();
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static STOPPING: OnceLock<ShutdownState> = OnceLock::new();

static ACTIVE_LABELS: LazyLock<Mutex<HashMap<Uuid, String>>> = LazyLock::new(Mutex::default);

/// Represents some activity which cannot be ruthlessly interrupted.
/// Obtain an Activity instance via Activity::get(). While any
/// Activity instances are alive in the program, LifeCycle::wait_for_shutdown
/// cannot complete.
pub struct Activity {
    tx: MPSCSender<()>,
    uuid: Uuid,
}

impl std::fmt::Debug for Activity {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("Activity").finish()
    }
}

impl Clone for Activity {
    fn clone(&self) -> Self {
        let uuid = Uuid::new_v4();
        let mut labels = ACTIVE_LABELS.lock().unwrap();

        let label = match labels.get(&self.uuid) {
            Some(existing) => format!("clone of {existing}"),
            None => format!("impossible missing label for {}", self.uuid),
        };
        labels.insert(uuid, label);

        Activity {
            tx: self.tx.clone(),
            uuid,
        }
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        ACTIVE_LABELS.lock().unwrap().remove(&self.uuid);
    }
}

impl Activity {
    /// Obtain an Activity instance.
    /// If None is returned then the process is shutting down
    /// and no new activity can be initiated.
    pub fn get_opt(label: String) -> Option<Self> {
        let uuid = Uuid::new_v4();
        let active = ACTIVE.get()?.lock().unwrap();
        let activity = active.as_ref()?;
        ACTIVE_LABELS.lock().unwrap().insert(uuid, label);
        Some(Activity {
            tx: activity.tx.clone(),
            uuid,
        })
    }

    /// Obtain an Activity instance.
    /// Returns Err if the process is shutting down and no new
    /// activity can be initiated
    pub fn get(label: String) -> anyhow::Result<Self> {
        Self::get_opt(label).ok_or_else(|| anyhow::anyhow!("shutting down"))
    }

    /// Returns true if the process is shutting down.
    pub fn is_shutting_down(&self) -> bool {
        SHUTTING_DOWN.load(Ordering::Relaxed)
    }
}

pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::Relaxed)
}

struct ShutdownState {
    tx: WatchSender<()>,
    rx: WatchReceiver<()>,
    request_shutdown_tx: MPSCSender<()>,
    stop_requested: AtomicBool,
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
        let uuid = Uuid::new_v4();
        ACTIVE_LABELS
            .lock()
            .unwrap()
            .insert(uuid, "Root LifeCycle".to_string());
        ACTIVE
            .set(Mutex::new(Some(Activity {
                tx: activity_tx,
                uuid,
            })))
            .map_err(|_| ())
            .unwrap();

        let (request_shutdown_tx, request_shutdown_rx) = tokio::sync::mpsc::channel(1);

        let (tx, rx) = tokio::sync::watch::channel(());
        STOPPING
            .set(ShutdownState {
                tx,
                rx,
                request_shutdown_tx,
                stop_requested: AtomicBool::new(false),
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
        tracing::debug!("shutdown has been requested");
        if let Some(state) = STOPPING.get() {
            if state.stop_requested.compare_exchange(
                false,
                true,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) == Ok(false)
            {
                state.request_shutdown_tx.send(()).await.ok();
            }
        } else {
            tracing::error!("request_shutdown: STOPPING channel is unavailable");
        }
    }

    /// Wait for a shutdown request, then propagate that state
    /// to running tasks, and then wait for those tasks to complete
    /// before returning to the caller.
    pub async fn wait_for_shutdown(&mut self) {
        // Wait for interrupt
        tracing::debug!("Waiting for interrupt");
        let mut sig_term =
            tokio::signal::unix::signal(SignalKind::terminate()).expect("listen for SIGTERM");
        let mut sig_hup =
            tokio::signal::unix::signal(SignalKind::hangup()).expect("listen for SIGUP");

        tokio::select! {
            _ = sig_term.recv() => {}
            _ = sig_hup.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
            _ = self.request_shutdown_rx.recv() => {}
        };
        tracing::debug!("wait_for_shutdown: shutdown requested!");
        tracing::info!(
            "Shutdown requested, please wait while in-flight messages are delivered \
             (based on your configured smtp client timeout duration) and \
             deferred spool messages are saved. \
             Interrupting shutdown may cause loss of message accountability \
             and/or duplicate delivery so please be patient!"
        );
        // Signal that we are stopping
        tracing::debug!("Signal tasks that we are stopping");
        SHUTTING_DOWN.store(true, Ordering::SeqCst);
        ACTIVE.get().map(|a| a.lock().unwrap().take());
        STOPPING.get().map(|s| s.tx.send(()).ok());
        // Wait for all pending activity to finish
        tracing::debug!("Waiting for tasks to wrap up");
        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {
                    let labels = ACTIVE_LABELS.lock().unwrap().clone();
                    let n = labels.len();
                    let summary :Vec<&str> = labels.values().map(|s| s.as_str()).take(10).collect();
                    let summary = summary.join(", ");
                    let summary = if labels.len() > 10 {
                        format!("{summary} (and {} others)", labels.len() - 10)
                    } else {
                        summary
                    };
                    tracing::info!("Still waiting for {n} pending activities... {summary}");
                }
                _ = self.activity_rx.recv() => {
                    return
                }
            }
        }
    }
}
