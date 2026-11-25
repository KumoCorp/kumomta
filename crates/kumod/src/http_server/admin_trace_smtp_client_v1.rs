use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use kumo_api_types::{TraceSmtpClientV1Event, TraceSmtpClientV1Payload, TraceSmtpClientV1Request};
use parking_lot::Mutex;
use rfc5321::DeferredTracer;
use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{channel, Receiver, Sender};
use tracing::Level;

static MGR: LazyLock<SmtpClientTraceManager> = LazyLock::new(SmtpClientTraceManager::new);

pub struct SmtpClientTraceManager {
    tx: Sender<SmtpClientTraceEvent>,
}

impl SmtpClientTraceManager {
    pub fn new() -> Self {
        let (tx, _rx) = channel(128 * 1024);
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
    fn to_v1(self, terse: bool) -> TraceSmtpClientV1Event {
        TraceSmtpClientV1Event {
            conn_meta: self.conn_meta,
            payload: self.payload.to_v1(terse),
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
    fn to_v1(self, terse: bool) -> TraceSmtpClientV1Payload {
        match self {
            Self::BeginSession => TraceSmtpClientV1Payload::BeginSession,
            Self::Connected => TraceSmtpClientV1Payload::Connected,
            Self::MessageObtained => TraceSmtpClientV1Payload::MessageObtained,
            Self::Closed => TraceSmtpClientV1Payload::Closed,
            Self::Read(data) => {
                TraceSmtpClientV1Payload::Read(String::from_utf8_lossy(&data).to_string())
            }
            Self::Write(s) => {
                if terse {
                    fn split_first_line(s: &str) -> Option<String> {
                        let mut iter = s.lines();
                        let snippet = iter.next()?;
                        iter.next()?;
                        Some(snippet.to_string())
                    }
                    if let Some(snippet) = split_first_line(&s) {
                        return TraceSmtpClientV1Payload::AbbreviatedWrite {
                            snippet,
                            len: s.len(),
                        };
                    }
                }
                TraceSmtpClientV1Payload::Write(s)
            }
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

fn mx_matches(meta: &serde_json::Value, request: &TraceSmtpClientV1Request) -> bool {
    if request.mx_host.is_empty() && request.mx_addr.is_none() {
        return true;
    }
    let matches = |peer: &serde_json::Value, host_key: &str, addr_key: &str| {
        let host_matches = request.mx_host.is_empty()
            || request
                .mx_host
                .iter()
                .any(|host| peer[host_key] == host.as_str());
        host_matches
            && request.mx_addr.as_ref().is_none_or(|cidrs| {
                addr_from_meta(peer, addr_key).is_some_and(|ip| cidrs.contains(ip))
            })
    };
    if meta.get("mx_address").is_some() {
        return matches(meta, "mx_host", "mx_address");
    }

    // Before the first connection, at least one planned MX must satisfy both filters.
    let plan = &meta["mx_plan"];
    if plan == "NullMx" {
        return false;
    }
    plan["Addresses"]
        .as_array()
        .is_none_or(|addresses| addresses.iter().any(|peer| matches(peer, "name", "addr")))
}

fn is_excluded(meta: &serde_json::Value, request: &TraceSmtpClientV1Request) -> bool {
    for (key, candidates) in [
        ("campaign", &request.campaign),
        ("domain", &request.domain),
        ("egress_pool", &request.egress_pool),
        ("egress_source", &request.egress_source),
        ("ready_queue_name", &request.ready_queue),
        ("recipient", &request.rcpt_to),
        ("routing_domain", &request.routing_domain),
        ("sender", &request.mail_from),
        ("tenant", &request.tenant),
    ] {
        if candidates.is_empty() {
            continue;
        }

        let matches = |value: &str| candidates.iter().any(|entry| entry == value);
        let matched = match meta.get(key) {
            Some(serde_json::Value::String(value)) => matches(value),
            Some(serde_json::Value::Array(values)) => values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(matches),
            // Before the first message these fields are unknown. Once a message is
            // selected, an unset field cannot match a requested value.
            _ if matches!(key, "campaign" | "tenant" | "routing_domain") => {
                meta.get("message_id").is_none()
            }
            _ => continue,
        };
        if !matched {
            return true;
        }
    }
    !mx_matches(meta, request)
}

async fn process_websocket_inner(
    mut socket: WebSocket,
    mut rx: Receiver<SmtpClientTraceEvent>,
) -> anyhow::Result<()> {
    let mut has_lagged = false;

    let request: TraceSmtpClientV1Request = match socket
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("websocket closed"))??
    {
        Message::Text(json) => serde_json::from_str(&json)?,
        message => anyhow::bail!("unexpected {message:?}"),
    };

    loop {
        let event = match rx.recv().await {
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
                let event = TraceSmtpClientV1Event {
                    conn_meta: serde_json::Value::Null,
                    when: Utc::now(),
                    payload: TraceSmtpClientV1Payload::Diagnostic {
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
            if let Some(src) = addr_from_meta(&event.conn_meta, "source_address") {
                if !cidrset.contains(src) {
                    continue;
                }
            }
        }

        if is_excluded(&event.conn_meta, &request) {
            continue;
        }

        let json = serde_json::to_string(&event.to_v1(request.terse))?;
        socket.send(Message::Text(json.into())).await?;
    }
}

async fn process_websocket(socket: WebSocket, rx: Receiver<SmtpClientTraceEvent>) {
    if let Err(err) = process_websocket_inner(socket, rx).await {
        tracing::error!("error in websocket: {err:#}");
    }
}

/// This is a websocket endpoint that provides outbound SMTP tracing.
/// It cannot be described via auto-generated docs extracted from the JSON Schema.
#[utoipa::path(get, tags = ["debugging", "kcli:trace-smtp-client"], path = "/api/admin/trace-smtp-client/v1")]
pub async fn trace(ws: WebSocketUpgrade) -> impl IntoResponse {
    // Subscribe before completing the upgrade handshake. The handshake
    // response is what unblocks the connecting client, so subscribing here
    // guarantees this session is counted as a receiver before the client can
    // return and start generating traceable activity; otherwise events
    // produced in that window would be dropped by the receiver-count check in
    // SmtpClientTraceManager::submit.
    let rx = MGR.tx.subscribe();
    ws.on_upgrade(move |socket| process_websocket(socket, rx))
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    #[test]
    fn mx_filters_match_one_candidate_before_connecting() {
        let plan = json!({"mx_plan": {"Addresses": [
            {"name": "first.example", "addr": "192.0.2.1"},
            {"name": "second.example", "addr": "[2001:db8::1]:25"}
        ]}});
        let by_host = |name: &str| TraceSmtpClientV1Request {
            mx_host: vec![name.into()],
            ..Default::default()
        };
        let by_addr = |cidr: &str| TraceSmtpClientV1Request {
            mx_addr: Some(vec![cidr].try_into().unwrap()),
            ..Default::default()
        };
        assert!(is_excluded(&plan, &by_host("other.example")));
        assert!(is_excluded(&plan, &by_addr("198.51.100.0/24")));
        let mut request = by_addr("2001:db8::/32");
        request.mx_host = vec!["first.example".into()];
        assert!(is_excluded(&plan, &request)); // Host and IP match different candidates.
        request.mx_host = vec!["second.example".into()];
        assert!(!is_excluded(&plan, &request));
        assert!(is_excluded(&json!({"mx_plan": "NullMx"}), &request));
        assert!(is_excluded(
            &json!({"mx_plan": {"Addresses": []}}),
            &request
        ));
    }

    #[test]
    fn connected_mx_takes_precedence_over_the_plan() {
        let request = TraceSmtpClientV1Request {
            mx_addr: Some(vec!["192.0.2.0/24"].try_into().unwrap()),
            ..Default::default()
        };
        let mut meta = json!({"mx_plan": {"Addresses": [
            {"name": "mx.example", "addr": "192.0.2.1"},
            {"name": "mx.example", "addr": "198.51.100.2"}
        ]}});
        assert!(!is_excluded(&meta, &request));
        meta["mx_host"] = json!("mx.example");
        meta["mx_address"] = json!("198.51.100.2");
        assert!(is_excluded(&meta, &request));
        meta["mx_address"] = json!("192.0.2.1");
        assert!(!is_excluded(&meta, &request));
    }

    #[test]
    fn ready_queue_filters_setup_events_before_recipients_are_known() {
        let request = TraceSmtpClientV1Request {
            ready_queue: vec!["wanted".into()],
            rcpt_to: vec!["first@example.test".into()],
            ..Default::default()
        };
        let excluded = |name| is_excluded(&json!({"ready_queue_name": name}), &request);
        assert!(!excluded("wanted"));
        assert!(excluded("other"));
    }

    #[test]
    fn optional_message_fields_must_match_once_a_message_is_selected() {
        let request = TraceSmtpClientV1Request {
            campaign: vec!["campaign".into()],
            tenant: vec!["tenant".into()],
            routing_domain: vec!["relay.example".into()],
            ..Default::default()
        };
        assert!(!is_excluded(&json!({}), &request));
        let message = json!({
            "message_id": "id", "campaign": "campaign", "tenant": "tenant", "routing_domain": "relay.example"
        });
        assert!(!is_excluded(&message, &request));
        for field in ["campaign", "tenant", "routing_domain"] {
            let mut missing = message.clone();
            if field == "routing_domain" {
                missing[field] = serde_json::Value::Null;
            } else {
                missing.as_object_mut().unwrap().remove(field);
            }
            assert!(is_excluded(&missing, &request), "missing {field}");
        }
    }

    #[test]
    fn rcpt_to_matches_any_message_recipient() {
        let request = TraceSmtpClientV1Request {
            rcpt_to: vec!["first@example.test".into(), "second@example.test".into()],
            ..Default::default()
        };
        let excluded =
            |recipients: &[&str]| is_excluded(&json!({"recipient": recipients}), &request);
        assert!(!excluded(&["other@example.test", "second@example.test"]));
        assert!(excluded(&["other@example.test"]));
        assert!(excluded(&[]));
    }
}
