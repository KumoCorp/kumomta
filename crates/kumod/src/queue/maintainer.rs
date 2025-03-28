use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::queue::insert_context::InsertReason;
use crate::queue::manager::{QueueManager, MANAGER};
use crate::queue::queue::{Queue, QueueHandle};
use crate::queue::wait_for_message_batch;
use crate::ready_queue::ReadyQueueManager;
use kumo_server_lifecycle::{Activity, ShutdownSubcription};
use kumo_server_runtime::Runtime;
use prometheus::IntCounter;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tracing::instrument;

const ONE_SECOND: Duration = Duration::from_secs(1);
const ONE_MINUTE: Duration = Duration::from_secs(60);
const ONE_DAY: Duration = Duration::from_secs(86400);

static QMAINT_THREADS: AtomicUsize = AtomicUsize::new(0);
pub static QMAINT_RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("qmaint", |cpus| cpus / 4, &QMAINT_THREADS).unwrap());

pub fn set_qmaint_threads(n: usize) {
    QMAINT_THREADS.store(n, Ordering::SeqCst);
}

pub static TOTAL_QMAINT_RUNS: LazyLock<IntCounter> = LazyLock::new(|| {
    prometheus::register_int_counter!(
        "total_qmaint_runs",
        "total number of times a scheduled queue maintainer was run"
    )
    .unwrap()
});

pub async fn queue_meta_maintainer() -> anyhow::Result<()> {
    let activity = Activity::get(format!("Queue Manager Meta Maintainer"))?;
    let mut shutdown = ShutdownSubcription::get();
    shutdown.shutting_down().await;
    loop {
        let names = QueueManager::all_queue_names();
        if names.is_empty() && ReadyQueueManager::number_of_queues() == 0 {
            tracing::debug!("All queues are reaped");
            drop(activity);
            return Ok(());
        }

        for name in names {
            if let Some(queue) = QueueManager::get_opt(&name) {
                for msg in queue.drain_timeq() {
                    Queue::save_if_needed_and_log(&msg, None).await;
                }

                if queue.is_empty() && ReadyQueueManager::number_of_queues() == 0 {
                    if MANAGER
                        .named
                        .remove_if(&name, |_key, _queue| {
                            queue.is_empty() && ReadyQueueManager::number_of_queues() == 0
                        })
                        .is_some()
                    {
                        tracing::debug!("{name}: there are no more queues and the scheduled queue is empty, reaping");
                    }
                }
            }
        }

        tokio::time::sleep(ONE_SECOND).await;
    }
}

/// Note that this is only spawned for QueueStrategy::TimerWheel
/// or QueueStrategy::SkipList.
/// The SingletonTimerWheel variants do not spawn this.
#[instrument(skip(q))]
pub async fn maintain_named_queue(q: &QueueHandle) -> anyhow::Result<()> {
    let mut shutdown = ShutdownSubcription::get();
    let mut next_item_due = Instant::now();

    loop {
        let sleeping = Instant::now();
        let reason = tokio::select! {
            _ = tokio::time::sleep_until(next_item_due.into()) => {"due"}
            _ = shutdown.shutting_down() => {"shutting_down"}
            _ = q.notify_maintainer.notified() => {"notified"}
        };

        TOTAL_QMAINT_RUNS.inc();

        {
            tracing::debug!(
                "maintaining {} {:?} which has {} entries. wakeup after {:?} reason={reason}",
                q.name,
                q.queue.strategy(),
                q.queue.len(),
                sleeping.elapsed(),
            );

            if let Some(b) = AdminBounceEntry::get_for_queue_name(&q.name) {
                q.bounce_all(&b).await;
            }

            if q.activity.is_shutting_down() {
                for msg in q.drain_timeq() {
                    Queue::save_if_needed_and_log(&msg, None).await;
                    drop(msg);
                }

                // Bow out and let the queue_meta_maintainer finish up
                return Ok(());
            }

            let (messages, next_due_in) = q.queue.pop();

            let now = Instant::now();

            next_item_due = if q.queue.is_timer_wheel() {
                // For a timer wheel, we need to (fairly consistently) tick it
                // over in order to promote things to the ready queue.
                // We do this based on the retry duration; the product default
                // is a 20m retry duration for which we want to tick once per
                // minute.
                // For shorter intervals we scale this accordingly.
                // To avoid very excessively wakeups for very short or very
                // long intervals, we clamp to between 1s and 1m.

                debug_assert!(
                    next_due_in.is_none(),
                    "next_due_in should never be populated for timerwheel"
                );

                let queue_config = q.queue_config.borrow();
                now + queue_config.timerwheel_tick_interval.unwrap_or(
                    (queue_config.retry_interval / 20)
                        .max(ONE_SECOND)
                        .min(ONE_MINUTE),
                )
            } else {
                now + next_due_in.unwrap_or(ONE_DAY)
            };

            if !messages.is_empty() {
                q.metrics().sub(messages.len());
                tracing::debug!("{} {} msgs are now ready", q.name, messages.len());

                wait_for_message_batch(&messages).await;

                for msg in messages {
                    q.insert_ready(msg, InsertReason::DueTimeWasReached.into(), None)
                        .await?;
                }
            }
        }
    }
}
