use crate::http_server::admin_bounce_v1::AdminBounceEntry;
use crate::queue::insert_context::InsertReason;
use crate::queue::manager::{QueueManager, MANAGER};
use crate::queue::queue::{Queue, QueueHandle};
use crate::queue::strategy::{QueueStructure, WheelV1Entry, SINGLETON_WHEEL, SINGLETON_WHEEL_V2};
use crate::queue::wait_for_message_batch;
use crate::ready_queue::ReadyQueueManager;
use anyhow::Context;
use kumo_server_lifecycle::{Activity, ShutdownSubcription};
use kumo_server_runtime::Runtime;
use message::message::MessageList;
use message::Message;
use parking_lot::FairMutex;
use prometheus::{Histogram, IntCounter};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{LazyLock, Once};
use std::time::{Duration, Instant};
use timeq::PopResult;
use tracing::instrument;

const ONE_SECOND: Duration = Duration::from_secs(1);
const ONE_MINUTE: Duration = Duration::from_secs(60);
const ONE_DAY: Duration = Duration::from_secs(86400);
static SPAWN_REINSERTION: AtomicBool = AtomicBool::new(false);

static QMAINT_THREADS: AtomicUsize = AtomicUsize::new(0);
pub static QMAINT_RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("qmaint", |cpus| cpus / 4, &QMAINT_THREADS).unwrap());

pub fn set_qmaint_threads(n: usize) {
    QMAINT_THREADS.store(n, Ordering::SeqCst);
}

pub fn set_spawn_reinsertion(v: bool) {
    SPAWN_REINSERTION.store(v, Ordering::SeqCst);
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
    tracing::trace!("queue_meta_maintainer: system is shutting down");
    loop {
        tracing::trace!("queue_meta_maintainer: get all scheduled queue names");
        let names = QueueManager::all_queue_names();
        tracing::trace!(
            "queue_meta_maintainer: got {} scheduled queue names",
            names.len()
        );

        if names.is_empty() && ReadyQueueManager::number_of_queues() == 0 {
            tracing::debug!("All queues are reaped");
            drop(activity);
            return Ok(());
        }

        for name in names {
            tracing::trace!("queue_meta_maintainer: examine {name}");
            if let Some(queue) = QueueManager::get_opt(&name) {
                tracing::trace!("queue_meta_maintainer: draining {name}");
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

        tracing::trace!("queue_meta_maintainer: sleep for a second");
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

async fn reinsert_ready(
    msg: Message,
    queue: QueueHandle,
    to_shrink: &mut HashMap<String, QueueHandle>,
) -> anyhow::Result<()> {
    if let Some(b) = AdminBounceEntry::get_for_queue_name(&queue.name) {
        // Note that this will cause the msg to be removed from the
        // queue so the remove() check below will return false
        queue.bounce_all(&b).await;
    }

    fn remove(q: &FairMutex<HashSet<Message>>, msg: &Message) -> bool {
        q.lock().remove(msg)
    }

    // Verify that the message is still in the queue
    match &queue.queue {
        QueueStructure::SingletonTimerWheel(q) | QueueStructure::SingletonTimerWheelV2(q) => {
            if remove(q, &msg) {
                queue.metrics().sub(1);
                queue
                    .insert_ready(msg, InsertReason::DueTimeWasReached.into(), None)
                    .await?;
                if !to_shrink.contains_key(queue.name.as_str()) {
                    to_shrink.insert(queue.name.to_string(), queue);
                }
            }
        }
        _ => {
            anyhow::bail!("impossible queue strategy");
        }
    }

    Ok(())
}

async fn reinsert_batch(messages: Vec<(Message, QueueHandle)>, total_scheduled: usize) {
    let mut to_shrink = HashMap::new();
    let mut reinserted = 0;

    let messages_only: Vec<Message> = messages.iter().map(|(msg, _q)| msg.clone()).collect();
    wait_for_message_batch(&messages_only).await;
    for (msg, queue) in messages {
        reinserted += 1;
        if let Err(err) = reinsert_ready(msg, queue, &mut to_shrink).await {
            tracing::error!("singleton_wheel: reinsert_ready: {err:#}");
        }
    }
    tracing::debug!(
        "singleton_wheel: done reinserting {reinserted}. total scheduled={total_scheduled}"
    );

    for (_queue_name, queue) in to_shrink.drain() {
        queue.queue.shrink();
    }
    to_shrink.shrink_to_fit();
}

async fn process_batch(messages: Vec<(Message, QueueHandle)>, total_scheduled: usize) {
    if messages.is_empty() {
        return;
    }

    if !SPAWN_REINSERTION.load(Ordering::Relaxed) {
        reinsert_batch(messages, total_scheduled).await;
        return;
    }

    if let Err(err) =
        QMAINT_RUNTIME.spawn("reinsert_batch", reinsert_batch(messages, total_scheduled))
    {
        tracing::error!("run_singleton_wheel_v1: failed to spawn reinsert_batch: {err:#}");
    }
}

async fn reinsert_ready_v2(
    msg: Message,
    to_shrink: &mut HashMap<String, QueueHandle>,
) -> anyhow::Result<()> {
    // Note that there is a potential race here that we cannot detect
    // until we try to call the v2 cancel method.
    // If we are no longer responsible for the message (detected later),
    // we might load the metadata here while a concurrent actor is requeing
    // the message and releasing the metadata.
    // That can cause the get_queue_name method to fail.
    // We address this in the v1 flavor of this function by keeping
    // the associate Queue handle so that we don't need to muck with the
    // metadata at all.
    // Here, we don't and can't do that.
    msg.load_meta_if_needed().await?;
    let queue_name = msg.get_queue_name().context("msg.get_queue_name")?;
    // Use get_opt rather than resolve here. If the queue is not currently
    // tracked in the QueueManager then this message cannot possibly belong
    // to it. Using resolve would have the side effect of creating an empty
    // queue for it, which will then age out later. It's a waste to do that,
    // so we just check and skip.
    let queue =
        QueueManager::get_opt(&queue_name).ok_or_else(|| anyhow::anyhow!("no scheduled queue"))?;

    if let Some(b) = AdminBounceEntry::get_for_queue_name(&queue.name) {
        // Note that this will cause the msg to be removed from the
        // queue so the remove() check below will return false
        queue.bounce_all(&b).await;
    }

    fn remove(q: &FairMutex<HashSet<Message>>, msg: &Message) -> bool {
        q.lock().remove(msg)
    }

    // Verify that the message is still in the queue
    match &queue.queue {
        QueueStructure::SingletonTimerWheel(q) | QueueStructure::SingletonTimerWheelV2(q) => {
            if remove(q, &msg) {
                queue.metrics().sub(1);
                queue
                    .insert_ready(msg, InsertReason::DueTimeWasReached.into(), None)
                    .await?;
                if !to_shrink.contains_key(queue.name.as_str()) {
                    to_shrink.insert(queue.name.to_string(), queue);
                }
            }
        }
        _ => {
            anyhow::bail!("impossible queue strategy");
        }
    }

    Ok(())
}

async fn reinsert_batch_v2(messages: Vec<Message>, total_scheduled: usize) {
    let mut to_shrink = HashMap::new();
    let mut reinserted = 0;

    wait_for_message_batch(&messages).await;
    for msg in messages {
        reinserted += 1;
        if let Err(err) = reinsert_ready_v2(msg, &mut to_shrink).await {
            tracing::error!("singleton_wheel: reinsert_ready: {err:#}");
        }
    }
    tracing::debug!(
        "singleton_wheel: done reinserting {reinserted}. total scheduled={total_scheduled}"
    );

    for (_queue_name, queue) in to_shrink.drain() {
        queue.queue.shrink();
    }
    to_shrink.shrink_to_fit();
}

async fn process_batch_v2(messages: Vec<Message>, total_scheduled: usize) {
    if messages.is_empty() {
        return;
    }

    if !SPAWN_REINSERTION.load(Ordering::Relaxed) {
        reinsert_batch_v2(messages, total_scheduled).await;
        return;
    }

    if let Err(err) = QMAINT_RUNTIME.spawn(
        "reinsert_batch",
        reinsert_batch_v2(messages, total_scheduled),
    ) {
        tracing::error!("run_singleton_wheel_v1: failed to spawn reinsert_batch: {err:#}");
    }
}

static POP_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "timeq_pop_latency",
        "The amount of time that passes between calls to a singleon timerwheel pop",
    )
    .unwrap()
});

async fn run_singleton_wheel_v1() -> anyhow::Result<()> {
    let mut shutdown = ShutdownSubcription::get();

    tracing::debug!("singleton_wheel_v1: starting up");

    loop {
        if tokio::time::timeout(Duration::from_secs(3), shutdown.shutting_down())
            .await
            .is_ok()
        {
            tracing::info!("singleton_wheel: stopping");
            return Ok(());
        }
        tracing::trace!("singleton_wheel_v1 ticking");
        TOTAL_QMAINT_RUNS.inc();

        fn pop() -> (Vec<WheelV1Entry>, usize) {
            let _timer = POP_LATENCY.start_timer();
            let mut wheel = SINGLETON_WHEEL.lock();

            let msgs = if let PopResult::Items(weak_messages) = wheel.pop() {
                tracing::debug!("singleton_wheel: popped {} messages", weak_messages.len());
                weak_messages
            } else {
                vec![]
            };

            (msgs, wheel.len())
        }

        let (msgs, total_scheduled) = pop();

        let mut messages = vec![];
        for weak_message in msgs {
            if let Some((msg, queue)) = weak_message.upgrade() {
                messages.push((msg, queue));
            }
        }
        process_batch(messages, total_scheduled).await;
    }
}

async fn run_singleton_wheel_v2() -> anyhow::Result<()> {
    let mut shutdown = ShutdownSubcription::get();

    tracing::debug!("singleton_wheel_v2: starting up");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            _ = shutdown.shutting_down() => {
                tracing::info!("singleton_wheel: stopping");
                return Ok(());
            }
        }
        tracing::trace!("singleton_wheel_v2 ticking");
        TOTAL_QMAINT_RUNS.inc();

        fn pop() -> (MessageList, usize) {
            let _timer = POP_LATENCY.start_timer();
            let mut wheel = SINGLETON_WHEEL_V2.lock();

            let ready = wheel.pop();
            if !ready.is_empty() {
                tracing::debug!("singleton_wheel_v2: popped {} messages", ready.len());
            }

            (ready, wheel.len())
        }

        let (messages, total_scheduled) = pop();
        process_batch_v2(messages.into_iter().collect(), total_scheduled).await;
    }
}

fn start_ticker(label: &'static str, f: impl std::future::Future<Output = ()> + Send + 'static) {
    if !SPAWN_REINSERTION.load(Ordering::SeqCst) {
        QMAINT_RUNTIME
            .spawn(format!("start_singleton_{label}"), f)
            .expect("failed to start ticker");
    } else {
        std::thread::Builder::new()
            .name(label.into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .expect("failed to build ticker runtime");
                runtime.block_on(f);
            })
            .expect("failed to spawn ticker");
    }
}

#[inline]
pub fn start_singleton_wheel_v1() {
    static STARTED_SINGLETON_WHEEL: Once = Once::new();
    STARTED_SINGLETON_WHEEL.call_once(|| {
        start_ticker("wheel_v1", async move {
            if let Err(err) = run_singleton_wheel_v1().await {
                tracing::error!("run_singleton_wheel_v1: {err:#}");
            }
        });
    });
}

#[inline]
pub fn start_singleton_wheel_v2() {
    static STARTED_SINGLETON_WHEEL_V2: Once = Once::new();
    STARTED_SINGLETON_WHEEL_V2.call_once(|| {
        start_ticker("wheel_v2", async move {
            if let Err(err) = run_singleton_wheel_v2().await {
                tracing::error!("run_singleton_wheel_v2: {err:#}");
            }
        });
    });
}
