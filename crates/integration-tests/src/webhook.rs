use axum::extract::{Extension, Json};
use axum::routing::post;
use axum::Router;
use axum_server::Handle;
use kumo_log_types::{JsonLogRecord, RecordType};
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct WebHookServer {
    pub addr: SocketAddr,
    pub records: Arc<Mutex<Vec<JsonLogRecord>>>,
    handle: Handle,
}

impl WebHookServer {
    pub async fn start() -> anyhow::Result<Self> {
        let records = Arc::new(Mutex::new(vec![]));

        let app = Router::new()
            .route("/log", post(log_record))
            .layer(Extension(Arc::clone(&records)));

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
            records,
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
                    while self.records.lock().unwrap().len() != count {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
            } => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }

    pub fn dump_logs(&self) -> anyhow::Result<BTreeMap<RecordType, usize>> {
        let records = self.records.lock().unwrap();

        let mut counts = BTreeMap::new();

        for record in records.iter() {
            *counts.entry(record.kind).or_default() += 1;
        }
        Ok(counts)
    }

    pub fn return_logs(&self) -> Vec<JsonLogRecord> {
        (*self.records.lock().unwrap()).clone()
    }
}

async fn log_record(
    Extension(records): Extension<Arc<Mutex<Vec<JsonLogRecord>>>>,
    Json(record): Json<JsonLogRecord>,
) {
    records.lock().unwrap().push(record);
}
