use super::trace_detail::{self, Subscription};
use crate::smtp_server::LogReportDisposition;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use kumo_api_types::TraceDetail::{self, *};
use kumo_api_types::{TraceSmtpV1Event, TraceSmtpV1Payload, TraceSmtpV1Request};
use kumo_server_common::http_server::TraceMetaKeyLevels;
use spool::SpoolId;
use std::hash::{BuildHasher, RandomState};
use std::net::IpAddr;
use std::sync::{Arc, LazyLock};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{channel, Receiver, Sender};
use uuid::Uuid;

static MGR: LazyLock<SmtpServerTraceManager> = LazyLock::new(|| SmtpServerTraceManager::new());

pub struct SmtpServerTraceManager {
    tx: Sender<SmtpServerTraceEvent>,
    // Each content subscription holds one clone for the life of its WebSocket.
    content_interest: Arc<()>,
}

#[derive(Clone)]
pub struct SmtpServerTraceEvent {
    pub session_id: Uuid,
    pub conn_meta: serde_json::Value,
    pub payload: SmtpServerTraceEventPayload,
    pub when: DateTime<Utc>,
}

impl SmtpServerTraceEvent {
    fn to_v1(
        mut self,
        detail: TraceDetail,
        terse: bool,
        ids: &RandomState,
        levels: &TraceMetaKeyLevels,
    ) -> Option<TraceSmtpV1Event> {
        let payload = self.payload.to_v1(detail, terse)?;
        trace_detail::project_meta(&mut self.conn_meta, detail, |k| {
            trace_detail::inbound_meta_detail(k, levels)
        });
        if detail == Protocol {
            self.conn_meta["id"] = format!("{:x}", ids.hash_one(self.session_id)).into();
        }
        Some(TraceSmtpV1Event {
            conn_meta: self.conn_meta,
            payload,
            when: self.when,
        })
    }
}

#[derive(Clone)]
pub enum SmtpServerTraceEventPayload {
    Connected,
    Closed,
    Read(Vec<u8>),
    DataRead(Vec<u8>),
    Command(Option<rfc5321::Command>),
    AuthContinuation,
    Data {
        len: usize,
        content: Option<Arc<[u8]>>,
    },
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
    pub fn data(bytes: &[u8]) -> Self {
        Self::Data {
            len: bytes.len(),
            content: MGR.capture_content(bytes),
        }
    }

    fn to_v1(self, detail: TraceDetail, terse: bool) -> Option<TraceSmtpV1Payload> {
        Some(match self {
            Self::Connected => TraceSmtpV1Payload::Connected,
            Self::Closed => TraceSmtpV1Payload::Closed,
            Self::Read(_) | Self::DataRead(_) if detail < Secrets => return None,
            Self::Command(_) | Self::AuthContinuation | Self::Data { .. } if detail == Secrets => {
                return None
            }
            Self::Command(cmd) => TraceSmtpV1Payload::Read(
                cmd.map(|c| trace_detail::command(&c, detail))
                    .unwrap_or_else(|| "[malformed command]".into()),
            ),
            Self::AuthContinuation => TraceSmtpV1Payload::Read("[AUTH continuation]".into()),
            Self::Data {
                content: Some(bytes),
                ..
            } if detail == Content => read_payload(&bytes, terse),
            Self::Data { len, .. } => TraceSmtpV1Payload::Read(format!("[{len} bytes]")),
            Self::Read(bytes) | Self::DataRead(bytes) => read_payload(&bytes, terse),
            Self::Write(s) => TraceSmtpV1Payload::Write(trace_detail::reply(s, detail)),
            Self::Callback { .. } | Self::Diagnostic { .. } if detail < Operational => return None,
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
                result: result.filter(|_| detail == Secrets),
                error,
            },
            Self::MessageDisposition { .. } if detail < Envelope => return None,
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
                meta: if detail == Secrets {
                    meta
                } else {
                    serde_json::Value::Null
                },
                sender,
                recipient,
                id,
                will_enqueue: Some(will_enqueue),
                was_arf_or_oob: Some(was_arf_or_oob),
            },
        })
    }
}

fn read_payload(data: &[u8], terse: bool) -> TraceSmtpV1Payload {
    if terse {
        if let Some(end) = data
            .trim_ascii_end()
            .iter()
            .position(|b| matches!(b, b'\r' | b'\n'))
        {
            return TraceSmtpV1Payload::AbbreviatedRead {
                snippet: String::from_utf8_lossy(&data[..end]).into_owned(),
                len: data.len(),
            };
        }
    }
    TraceSmtpV1Payload::Read(String::from_utf8_lossy(data).into_owned())
}

impl SmtpServerTraceManager {
    pub fn new() -> Self {
        let (tx, _rx) = channel(128 * 1024);
        Self {
            tx,
            content_interest: Arc::new(()),
        }
    }

    fn subscribe(&self, detail: TraceDetail) -> (Receiver<SmtpServerTraceEvent>, Option<Arc<()>>) {
        let interest = (detail == Content).then(|| self.content_interest.clone());
        (self.tx.subscribe(), interest)
    }

    fn capture_content(&self, bytes: &[u8]) -> Option<Arc<[u8]>> {
        (Arc::strong_count(&self.content_interest) > 1).then(|| Arc::from(bytes))
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

async fn process_websocket_inner(
    mut socket: WebSocket,
    mut rx: Receiver<SmtpServerTraceEvent>,
    subscription: Subscription,
) -> anyhow::Result<()> {
    let ids = RandomState::new();
    let mut has_lagged = false;

    let request: TraceSmtpV1Request = match socket
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("websocket closed"))??
    {
        Message::Text(json) => serde_json::from_str(&json)?,
        message => anyhow::bail!("unexpected {message:?}"),
    };
    if request.source_addr.is_some() && !subscription.allow_filter(Operational, &mut socket).await?
    {
        return Ok(());
    }

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

                let is_data = matches!(event.payload, SmtpServerTraceEventPayload::Data { .. } | SmtpServerTraceEventPayload::DataRead(_));
                let Some(event) = event.to_v1(subscription.detail, request.terse, &ids, &subscription.inbound_meta) else { continue };
                let mut json = serde_json::to_string(&event)?;
                if is_data && json.len() > trace_detail::MAX_FRAME_SIZE {
                    json = trace_detail::omitted_data(event.when, event.conn_meta)?;
                }
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

async fn process_websocket(
    socket: WebSocket,
    rx: Receiver<SmtpServerTraceEvent>,
    subscription: Subscription,
) {
    if let Err(err) = process_websocket_inner(socket, rx, subscription).await {
        tracing::error!("error in websocket: {err:#}");
    }
}

/// This is a websocket endpoint that provides inbound SMTP tracing.
/// It cannot be described via auto-generated docs extracted from the JSON Schema.
#[utoipa::path(get, tags=["debugging", "kcli:trace-smtp-server"], path = "/api/admin/trace-smtp-server/v1")]
pub async fn trace(subscription: Subscription, ws: WebSocketUpgrade) -> impl IntoResponse {
    // Subscribe before completing the upgrade handshake. The handshake
    // response is what unblocks the connecting client, so subscribing here
    // guarantees this session is counted as a receiver before the client can
    // return and start generating traceable activity; otherwise events
    // produced in that window would be dropped by the receiver-count check in
    // SmtpServerTraceManager::submit.
    let (rx, content_interest) = MGR.subscribe(subscription.detail);
    (
        [("kumo-trace-detail", subscription.detail.to_string())],
        ws.on_upgrade(move |socket| async move {
            let _interest = content_interest;
            process_websocket(socket, rx, subscription).await
        }),
    )
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn protocol_alias_tracks_session_not_endpoint_addresses() {
        let ids = RandomState::new();
        let alias = |session_id, from: &str, via: &str| {
            let event = SmtpServerTraceEvent {
                session_id,
                conn_meta: serde_json::json!({"received_from": from, "received_via": via}),
                payload: SmtpServerTraceEventPayload::Connected,
                when: Utc::now(),
            };
            event
                .to_v1(Protocol, false, &ids, &Default::default())
                .unwrap()
                .conn_meta["id"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let session = Uuid::new_v4();
        let original = alias(session, "192.0.2.1:1234", "192.0.2.2:25");
        assert_eq!(original, alias(session, "192.0.2.42:9999", "192.0.2.4:587"));
        assert_ne!(
            original,
            alias(Uuid::new_v4(), "192.0.2.1:1234", "192.0.2.2:25")
        );
    }

    #[test]
    fn terse_data_previews_the_first_line_with_bare_lf() {
        let data = b"Subject: preview\n\nbody\r\n";
        assert_eq!(
            read_payload(data, true),
            TraceSmtpV1Payload::AbbreviatedRead {
                snippet: "Subject: preview".into(),
                len: data.len(),
            }
        );
    }

    #[test]
    fn framed_body_copy_requires_a_content_subscriber() {
        let mgr = SmtpServerTraceManager::new();
        let _others =
            [Protocol, Operational, Envelope, Secrets].map(|detail| mgr.subscribe(detail));
        assert!(mgr.capture_content(b"body").is_none());
        let first = mgr.subscribe(Content);
        let second = mgr.subscribe(Content);
        drop(first);
        assert_eq!(
            mgr.capture_content(b"body").as_deref(),
            Some(b"body".as_slice())
        );
        drop(second);
        assert!(mgr.capture_content(b"body").is_none());
    }
}
