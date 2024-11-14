#![cfg(test)]
use axum::extract::{Extension, Json};
use axum::routing::post;
use axum::Router;
use axum_server::Handle;
use kumo_log_types::{JsonLogRecord, RecordType};
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Shared {
    records: Vec<JsonLogRecord>,
    request_counter: usize,
}

pub struct WebHookServer {
    pub addr: SocketAddr,
    shared: Arc<Mutex<Shared>>,
    handle: Handle,
}

impl WebHookServer {
    pub async fn start() -> anyhow::Result<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));

        let app = Router::new()
            .route("/log", post(log_record))
            .route("/log-batch", post(log_batch))
            .layer(Extension(Arc::clone(&shared)));

        let handle = Handle::new();

        let socket = TcpListener::bind("127.0.0.1:0")?;
        let addr = socket.local_addr()?;

        let server = axum_server::from_tcp(socket);
        let handle_copy = handle.clone();
        tokio::spawn(async move {
            server
                .handle(handle_copy)
                .serve(app.into_make_service())
                .await
                .unwrap();
        });

        Ok(Self {
            addr,
            shared,
            handle,
        })
    }

    pub fn shutdown(&self) {
        self.handle.shutdown();
    }

    pub async fn wait_for_record_count(&self, count: usize, timeout: Duration) -> bool {
        eprintln!("waiting for webhook records to populate");

        tokio::select! {
            _ = async {
                    while self.shared.lock().unwrap().records.len() != count {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
            } => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }

    pub fn request_counter(&self) -> usize {
        self.shared.lock().unwrap().request_counter
    }

    pub fn dump_logs(&self) -> anyhow::Result<BTreeMap<RecordType, usize>> {
        let records = self.shared.lock().unwrap().records.clone();

        let mut counts = BTreeMap::new();

        for record in records {
            *counts.entry(record.kind).or_default() += 1;
        }
        Ok(counts)
    }

    pub fn return_logs(&self) -> Vec<JsonLogRecord> {
        self.shared.lock().unwrap().records.clone()
    }
}

async fn log_batch(
    Extension(shared): Extension<Arc<Mutex<Shared>>>,
    Json(mut batch): Json<Vec<JsonLogRecord>>,
) {
    let mut shared = shared.lock().unwrap();
    shared.records.append(&mut batch);
    shared.request_counter += 1;
}

async fn log_record(
    Extension(shared): Extension<Arc<Mutex<Shared>>>,
    Json(record): Json<JsonLogRecord>,
) {
    let mut shared = shared.lock().unwrap();
    shared.records.push(record);
    shared.request_counter += 1;
}
