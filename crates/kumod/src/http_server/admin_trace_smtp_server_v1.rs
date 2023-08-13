use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use kumo_api_types::{TraceSmtpV1Event, TraceSmtpV1Payload, TraceSmtpV1Request};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use once_cell::sync::Lazy;
use spool::SpoolId;
use std::net::IpAddr;
use tokio::sync::broadcast::{channel, Sender};

static MGR: Lazy<SmtpServerTraceManager> = Lazy::new(|| SmtpServerTraceManager::new());

pub struct SmtpServerTraceManager {
    tx: Sender<SmtpServerTraceEvent>,
}

#[derive(Clone)]
pub struct SmtpServerTraceEvent {
    pub conn_meta: serde_json::Value,
    pub payload: SmtpServerTraceEventPayload,
    pub when: DateTime<Utc>,
}

impl SmtpServerTraceEvent {
    fn to_v1(self) -> TraceSmtpV1Event {
        TraceSmtpV1Event {
            conn_meta: self.conn_meta,
            payload: self.payload.to_v1(),
            when: self.when,
        }
    }
}

#[derive(Clone)]
pub enum SmtpServerTraceEventPayload {
    Connected,
    Closed,
    Read(Vec<u8>),
    Write(String),
    Diagnostic {
        level: tracing::Level,
        message: String,
    },
    Callback {
        name: String,
        result: Option<serde_json::Value>,
        error: Option<String>,
    },
    MessageDisposition {
        relay: bool,
        log_arf: bool,
        log_oob: bool,
        queue: String,
        meta: serde_json::Value,
        sender: String,
        recipient: String,
        id: SpoolId,
    },
}

impl SmtpServerTraceEventPayload {
    fn to_v1(self) -> TraceSmtpV1Payload {
        match self {
            Self::Connected => TraceSmtpV1Payload::Connected,
            Self::Closed => TraceSmtpV1Payload::Closed,
            Self::Read(data) => {
                TraceSmtpV1Payload::Read(String::from_utf8_lossy(&data).to_string())
            }
            Self::Write(s) => TraceSmtpV1Payload::Write(s),
            Self::Diagnostic { level, message } => TraceSmtpV1Payload::Diagnostic {
                level: level.to_string(),
                message: message.to_string(),
            },
            Self::Callback {
                name,
                result,
                error,
            } => TraceSmtpV1Payload::Callback {
                name,
                result,
                error,
            },
            Self::MessageDisposition {
                relay,
                log_arf,
                log_oob,
                queue,
                meta,
                sender,
                recipient,
                id,
            } => TraceSmtpV1Payload::MessageDisposition {
                relay,
                log_arf,
                log_oob,
                queue,
                meta,
                sender,
                recipient,
                id,
            },
        }
    }
}

impl SmtpServerTraceManager {
    pub fn new() -> Self {
        let (tx, _rx) = channel(16);
        Self { tx }
    }

    /// Submit a trace event to any connected trace subscribers.
    /// This is implemented via a closure so that the work to construct
    /// the event can be skipped if there are no subscribers
    pub fn submit<F: FnOnce() -> SmtpServerTraceEvent>(f: F) {
        let mgr = &MGR;
        if mgr.tx.receiver_count() > 0 {
            mgr.tx.send((f)()).ok();
        }
    }
}

fn peer_from_meta(meta: &serde_json::Value) -> Option<IpAddr> {
    let peer = meta.get("received_from")?;
    let peer = peer.as_str()?;

    if let Some((ip, _port)) = peer.rsplit_once(':') {
        ip.parse().ok()
    } else {
        peer.parse().ok()
    }
}

async fn process_websocket_inner(mut socket: WebSocket) -> anyhow::Result<()> {
    let mut rx = MGR.tx.subscribe();

    let request: TraceSmtpV1Request = match socket
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
            if let Some(peer) = peer_from_meta(&event.conn_meta) {
                if !cidrset.contains(peer) {
                    continue;
                }
            }
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
