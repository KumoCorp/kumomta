use crate::smtp_server::LogReportDisposition;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use kumo_api_types::{TraceSmtpV1Event, TraceSmtpV1Payload, TraceSmtpV1Request};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use spool::SpoolId;
use std::net::IpAddr;
use std::sync::LazyLock;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{channel, Sender};

static MGR: LazyLock<SmtpServerTraceManager> = LazyLock::new(|| SmtpServerTraceManager::new());

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
    fn to_v1(self, terse: bool) -> TraceSmtpV1Event {
        TraceSmtpV1Event {
            conn_meta: self.conn_meta,
            payload: self.payload.to_v1(terse),
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
        log_arf: LogReportDisposition,
        log_oob: LogReportDisposition,
        will_enqueue: bool,
        was_arf_or_oob: bool,
        queue: String,
        meta: serde_json::Value,
        sender: String,
        recipient: Vec<String>,
        id: SpoolId,
    },
}

impl SmtpServerTraceEventPayload {
    fn to_v1(self, terse: bool) -> TraceSmtpV1Payload {
        match self {
            Self::Connected => TraceSmtpV1Payload::Connected,
            Self::Closed => TraceSmtpV1Payload::Closed,
            Self::Read(data) => {
                if terse {
                    fn split_first_line(slice: &[u8]) -> Option<String> {
                        let mut iter = slice.trim_ascii_end().split(|&b| b == b'\r');
                        let snippet = iter.next()?;
                        iter.next()?;
                        Some(String::from_utf8_lossy(snippet).to_string())
                    }

                    if let Some(snippet) = split_first_line(&data) {
                        return TraceSmtpV1Payload::AbbreviatedRead {
                            snippet,
                            len: data.len(),
                        };
                    }
                }

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
                will_enqueue,
                queue,
                meta,
                sender,
                recipient,
                id,
                was_arf_or_oob,
            } => TraceSmtpV1Payload::MessageDisposition {
                relay,
                log_arf: log_arf.into(),
                log_oob: log_oob.into(),
                queue,
                meta,
                sender,
                recipient,
                id,
                will_enqueue: Some(will_enqueue),
                was_arf_or_oob: Some(was_arf_or_oob),
            },
        }
    }
}

impl SmtpServerTraceManager {
    pub fn new() -> Self {
        let (tx, _rx) = channel(128 * 1024);
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
    let mut has_lagged = false;

    let request: TraceSmtpV1Request = match socket
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("websocket closed"))??
    {
        Message::Text(json) => serde_json::from_str(&json)?,
        message => anyhow::bail!("unexpected {message:?}"),
    };

    loop {
        tokio::select! {
            event = rx.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(RecvError::Closed) => {
                        return Ok(());
                    }
                    Err(RecvError::Lagged(n)) => {
                        let message = format!("Tracer Lagged behind and missed {n} events");

                        if !has_lagged {
                            tracing::error!(
                                "{message} (this message is shown only once per trace session)"
                            );
                            has_lagged = true;
                        }
                        let event = TraceSmtpV1Event {
                            conn_meta: serde_json::Value::Null,
                            when: Utc::now(),
                            payload: TraceSmtpV1Payload::Diagnostic {
                                level: tracing::Level::ERROR.to_string(),
                                message,
                            },
                        };
                        let json = serde_json::to_string(&event)?;
                        socket.send(Message::Text(json.into())).await?;
                        continue;
                    }
                };
                if let Some(cidrset) = &request.source_addr {
                    if let Some(peer) = peer_from_meta(&event.conn_meta) {
                        if !cidrset.contains(peer) {
                            continue;
                        }
                    }
                }

                let json = serde_json::to_string(&event.to_v1(request.terse))?;
                socket.send(Message::Text(json.into())).await?;
            }

            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_)))=> {
                        return Ok(());
                    }
                    Some(Ok(Message::Ping(ping))) => {
                        socket.send(Message::Pong(ping)).await?;
                    }
                    Some(Ok(Message::Pong(_)))=> {
                        continue;
                    }
                    Some(Ok(Message::Text(_) | Message::Binary(_)))=> {
                        tracing::error!("Received unexpected {msg:?} from client");
                        return Ok(());
                    }
                    Some(Err(err)) => {
                        tracing::error!("{err:#}, closing trace session");
                    }
                    None => {
                        return Ok(());
                    }
                }
            }
        }
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
