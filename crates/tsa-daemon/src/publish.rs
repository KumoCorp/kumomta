use crate::http_server::{open_history_db, publish_log_batch};
use kumo_log_types::JsonLogRecord;
use parking_lot::Mutex;
use std::sync::LazyLock;
use tokio::sync::Notify;
use tokio::task::LocalSet;
use tokio::time::{Duration, Instant};

const BATCH_SIZE: usize = 1024;
const MAX_BACKLOG: usize = 128 * 1024;
const BATCH_DURATION: Duration = Duration::from_millis(100);

struct Segment {
    started: Instant,
    contents: Vec<JsonLogRecord>,
}

#[derive(Default)]
struct BatchQueue {
    segments: Vec<Segment>,
    size: usize,
}

impl BatchQueue {
    fn push(&mut self, record: JsonLogRecord) -> bool {
        let new_seg = self
            .segments
            .last()
            .map(|seg| seg.contents.len() == BATCH_SIZE)
            .unwrap_or(true);
        if new_seg {
            let mut contents = Vec::with_capacity(BATCH_SIZE);
            contents.push(record);
            self.segments.push(Segment {
                started: Instant::now(),
                contents,
            });
        } else {
            self.segments
                .last_mut()
                .map(move |seg| seg.contents.push(record));
        }

        self.size += 1;

        let should_notify = self.segments.len() > 1
            || self
                .segments
                .last()
                .map(|seg| {
                    seg.contents.len() == BATCH_SIZE || seg.started.elapsed() >= BATCH_DURATION
                })
                .unwrap_or(false);

        should_notify
    }

    fn should_flush_based_on_time(&self) -> bool {
        self.segments
            .iter()
            .any(|seg| seg.started.elapsed() >= BATCH_DURATION)
    }
}

static NOTIFY_CONSUMER: LazyLock<Notify> = LazyLock::new(Notify::new);
static NOTIFY_PRODUCER: LazyLock<Notify> = LazyLock::new(Notify::new);
static QUEUE: LazyLock<Mutex<BatchQueue>> = LazyLock::new(|| start_processor_pool().unwrap());

fn try_push(record: JsonLogRecord) -> Result<bool, JsonLogRecord> {
    let mut queue = QUEUE.lock();
    if queue.size >= MAX_BACKLOG {
        Err(record)
    } else {
        Ok(queue.push(record))
    }
}

pub async fn submit_record(mut record: JsonLogRecord) -> anyhow::Result<()> {
    loop {
        match try_push(record) {
            Ok(should_notify) => {
                if should_notify {
                    NOTIFY_CONSUMER.notify_waiters();
                }
                return Ok(());
            }
            Err(rec) => {
                record = rec;
                NOTIFY_CONSUMER.notify_waiters();
                tracing::warn!("backlog hit, waiting");
                NOTIFY_PRODUCER.notified().await;
                tracing::debug!("after backlog wait");
            }
        }
    }
}

fn grab_segment() -> Option<Vec<JsonLogRecord>> {
    let mut queue = QUEUE.lock();
    if queue.segments.len() > 1 {
        let segment = queue.segments.remove(0);
        queue.size -= segment.contents.len();
        NOTIFY_PRODUCER.notify_one();
        return Some(segment.contents);
    }
    if queue
        .segments
        .last()
        .map(|seg| seg.contents.len() >= BATCH_SIZE || seg.started.elapsed() >= BATCH_DURATION)
        .unwrap_or(false)
    {
        let segment = queue.segments.pop()?;
        queue.size -= segment.contents.len();
        NOTIFY_PRODUCER.notify_one();
        return Some(segment.contents);
    }
    None
}

async fn run_processor() {
    let db = open_history_db().unwrap();
    loop {
        NOTIFY_CONSUMER.notified().await;
        while let Some(mut batch) = grab_segment() {
            if let Err(err) = publish_log_batch(&db, &mut batch).await {
                tracing::error!("Error in publish_log_v1_impl: {err:#}");
            }
        }
    }
}

async fn flush_batches() {
    loop {
        tokio::time::sleep(BATCH_DURATION).await;
        if QUEUE.lock().should_flush_based_on_time() {
            NOTIFY_CONSUMER.notify_waiters();
        }
    }
}

fn start_processor_pool() -> anyhow::Result<Mutex<BatchQueue>> {
    let n_threads: usize = std::thread::available_parallelism()?.into();

    for i in 0..n_threads {
        std::thread::Builder::new()
            .name(format!("processor-{i}"))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .on_thread_park(|| kumo_server_memory::purge_thread_cache())
                    .build()
                    .unwrap();
                let local_set = LocalSet::new();

                local_set.block_on(&runtime, run_processor());
            })?;
    }
    tokio::spawn(flush_batches());

    Ok(Mutex::new(BatchQueue::default()))
}
