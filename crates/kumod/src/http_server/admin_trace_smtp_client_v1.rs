use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use kumo_api_types::{TraceSmtpClientV1Event, TraceSmtpClientV1Payload, TraceSmtpClientV1Request};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use parking_lot::Mutex;
use rfc5321::DeferredTracer;
use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use tokio::sync::broadcast::{channel, Sender};
use tracing::Level;

static MGR: LazyLock<SmtpClientTraceManager> = LazyLock::new(SmtpClientTraceManager::new);

pub struct SmtpClientTraceManager {
    tx: Sender<SmtpClientTraceEvent>,
}

impl SmtpClientTraceManager {
    pub fn new() -> Self {
        let (tx, _rx) = channel(128);
        Self { tx }
    }

    /// Submit a trace event to any connected trace subscribers.
    /// This is implemented via a closure so that the work to construct
    /// the event can be skipped if there are no subscribers
    pub fn submit<F: FnOnce() -> SmtpClientTraceEvent>(f: F) {
        let mgr = &MGR;
        if mgr.tx.receiver_count() > 0 {
            mgr.tx.send((f)()).ok();
        }
    }
}

pub struct SmtpClientTracerImpl {
    meta: Mutex<serde_json::Value>,
}

impl std::fmt::Debug for SmtpClientTracerImpl {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let meta = self.meta.lock();
        match serde_json::to_string(&*meta) {
            Ok(s) => write!(fmt, "SmtpClientTracerImpl(meta={s})"),
            Err(_) => fmt
                .debug_struct("SmtpClientTracerImpl")
                .field("meta", &meta)
                .finish(),
        }
    }
}

impl SmtpClientTracerImpl {
    pub fn new(meta: serde_json::Value) -> Self {
        SmtpClientTraceManager::submit(|| SmtpClientTraceEvent {
            conn_meta: meta.clone(),
            payload: SmtpClientTraceEventPayload::BeginSession,
            when: Utc::now(),
        });

        Self {
            meta: Mutex::new(meta),
        }
    }

    pub fn set_meta<V: Into<serde_json::Value>>(&self, key: &str, value: V) {
        self.meta.lock()[key] = value.into();
    }

    pub fn unset_meta(&self, key: &str) {
        let mut map = self.meta.lock();
        match &mut *map {
            serde_json::Value::Object(map) => {
                map.remove(key);
            }
            _ => {}
        }
    }

    fn clone_meta(&self) -> serde_json::Value {
        self.meta.lock().clone()
    }

    pub fn submit<F: FnOnce() -> SmtpClientTraceEventPayload>(&self, f: F) {
        SmtpClientTraceManager::submit(|| SmtpClientTraceEvent {
            conn_meta: self.clone_meta(),
            payload: f(),
            when: Utc::now(),
        })
    }

    pub fn diagnostic<F: FnOnce() -> String>(&self, level: Level, msg_func: F) {
        self.submit(|| SmtpClientTraceEventPayload::Diagnostic {
            level,
            message: msg_func(),
        })
    }
}

fn port_payload(event: rfc5321::SmtpClientTraceEvent) -> SmtpClientTraceEventPayload {
    use rfc5321::SmtpClientTraceEvent as TE;
    match event {
        TE::Closed => SmtpClientTraceEventPayload::Closed,
        TE::Read(data) => SmtpClientTraceEventPayload::Read(data),
        TE::Write(data) => SmtpClientTraceEventPayload::Write(data),
        TE::Diagnostic { level, message } => {
            SmtpClientTraceEventPayload::Diagnostic { level, message }
        }
    }
}

impl rfc5321::SmtpClientTracer for SmtpClientTracerImpl {
    fn trace_event(&self, event: rfc5321::SmtpClientTraceEvent) {
        SmtpClientTraceManager::submit(|| {
            let payload = port_payload(event);
            SmtpClientTraceEvent {
                conn_meta: self.meta.lock().clone(),
                payload,
                when: Utc::now(),
            }
        });
    }

    fn lazy_trace(&self, deferred: &dyn DeferredTracer) {
        SmtpClientTraceManager::submit(|| {
            let payload = port_payload(deferred.trace());
            SmtpClientTraceEvent {
                conn_meta: self.meta.lock().clone(),
                payload,
                when: Utc::now(),
            }
        });
    }
}

#[derive(Clone, Debug)]
pub struct SmtpClientTraceEvent {
    pub conn_meta: serde_json::Value,
    pub payload: SmtpClientTraceEventPayload,
    pub when: DateTime<Utc>,
}

impl SmtpClientTraceEvent {
    fn to_v1(self) -> TraceSmtpClientV1Event {
        TraceSmtpClientV1Event {
            conn_meta: self.conn_meta,
            payload: self.payload.to_v1(),
            when: self.when,
        }
    }
}

#[derive(Clone, Debug)]
pub enum SmtpClientTraceEventPayload {
    BeginSession,
    Connected,
    Closed,
    Read(Vec<u8>),
    Write(String),
    Diagnostic {
        level: tracing::Level,
        message: String,
    },
    MessageObtained,
}

impl SmtpClientTraceEventPayload {
    fn to_v1(self) -> TraceSmtpClientV1Payload {
        match self {
            Self::BeginSession => TraceSmtpClientV1Payload::BeginSession,
            Self::Connected => TraceSmtpClientV1Payload::Connected,
            Self::MessageObtained => TraceSmtpClientV1Payload::MessageObtained,
            Self::Closed => TraceSmtpClientV1Payload::Closed,
            Self::Read(data) => {
                TraceSmtpClientV1Payload::Read(String::from_utf8_lossy(&data).to_string())
            }
            Self::Write(s) => TraceSmtpClientV1Payload::Write(s),
            Self::Diagnostic { level, message } => TraceSmtpClientV1Payload::Diagnostic {
                level: level.to_string(),
                message: message.to_string(),
            },
        }
    }
}

fn addr_from_meta(meta: &serde_json::Value, key: &str) -> Option<IpAddr> {
    let addr = meta.get(key)?;
    let addr = addr.as_str()?;

    match addr.parse::<SocketAddr>() {
        Ok(addr) => Some(addr.ip()),
        Err(_) => addr.parse::<IpAddr>().ok(),
    }
}

fn is_excluded(meta: &serde_json::Value, entries: &[(&str, &[String])]) -> bool {
    for (key, candidates) in entries {
        if candidates.is_empty() {
            continue;
        }

        if let Some(value) = meta.get(key).and_then(|v| v.as_str()) {
            if !candidates.iter().any(|entry| entry == value) {
                return true;
            }
        }
    }
    false
}

async fn process_websocket_inner(mut socket: WebSocket) -> anyhow::Result<()> {
    let mut rx = MGR.tx.subscribe();

    let request: TraceSmtpClientV1Request = match socket
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("websocket closed"))??
    {
        Message::Text(json) => serde_json::from_str(&json)?,
        message => anyhow::bail!("unexpected {message:?}"),
    };

    loop {
        let event = rx.recv().await?;

        if let Some(cidrset) = &request.source_addr {
            if let Some(src) = addr_from_meta(&event.conn_meta, "source_address") {
                if !cidrset.contains(src) {
                    continue;
                }
            }
        }
        if let Some(cidrset) = &request.mx_addr {
            if let Some(src) = addr_from_meta(&event.conn_meta, "mx_address") {
                if !cidrset.contains(src) {
                    continue;
                }
            }
        }

        if is_excluded(
            &event.conn_meta,
            &[
                ("campaign", &request.campaign),
                ("domain", &request.domain),
                ("egress_pool", &request.egress_pool),
                ("egress_source", &request.egress_source),
                ("mx_host", &request.mx_host),
                ("ready_queue", &request.ready_queue),
                ("recipient", &request.rcpt_to),
                ("routing_domain", &request.routing_domain),
                ("sender", &request.mail_from),
                ("tenant", &request.tenant),
            ],
        ) {
            continue;
        }

        let json = serde_json::to_string(&event.to_v1())?;
        socket.send(Message::Text(json)).await?;
    }
}

async fn process_websocket(socket: WebSocket) {
    if let Err(err) = process_websocket_inner(socket).await {
        tracing::error!("error in websocket: {err:#}");
    }
}

pub async fn trace(_: TrustedIpRequired, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|socket| process_websocket(socket))
}
