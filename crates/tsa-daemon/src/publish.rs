use crate::http_server::{open_history_db, publish_log_batch};
use batch_channel::{Receiver, Sender};
use kumo_log_types::JsonLogRecord;
use std::sync::LazyLock;
use tokio::task::LocalSet;

static PROCESSOR: LazyLock<Sender<JsonLogRecord>> =
    LazyLock::new(|| start_processor_pool().unwrap());

pub async fn submit_record(record: JsonLogRecord) -> anyhow::Result<()> {
    PROCESSOR.send(record).await?;
    Ok(())
}

async fn run_processor(rx: Receiver<JsonLogRecord>) {
    const BATCH_SIZE: usize = 1024;
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let db = open_history_db().unwrap();
    loop {
        rx.recv_vec(BATCH_SIZE, &mut batch).await;
        if batch.is_empty() {
            // All Senders are dropped
            return;
        }

        if let Err(err) = publish_log_batch(&db, &mut batch).await {
            tracing::error!("Error in publish_log_v1_impl: {err:#}");
        }
    }
}

pub fn start_processor_pool() -> anyhow::Result<Sender<JsonLogRecord>> {
    let n_threads: usize = std::thread::available_parallelism()?.into();
    let (tx, rx) = batch_channel::bounded(128 * 1024);

    for i in 0..n_threads {
        let rx = rx.clone();
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

                local_set.block_on(&runtime, run_processor(rx));
            })?;
    }

    Ok(tx)
}
