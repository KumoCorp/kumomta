use super::trace_detail::{self, Subscription};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use kumo_api_types::TraceDetail::{self, *};
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
        TE::Command(cmd) => SmtpClientTraceEventPayload::Command(cmd),
        TE::Data(bytes) => SmtpClientTraceEventPayload::Data(bytes),
        TE::Reply(line) => SmtpClientTraceEventPayload::Reply(line),
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
    fn to_v1(mut self, detail: TraceDetail, terse: bool) -> Option<TraceSmtpClientV1Event> {
        let payload = self.payload.to_v1(detail, terse)?;
        trace_detail::project_meta(
            &mut self.conn_meta,
            detail,
            trace_detail::outbound_meta_detail,
        );
        Some(TraceSmtpClientV1Event {
            conn_meta: self.conn_meta,
            payload,
            when: self.when,
        })
    }
}

#[derive(Clone, Debug)]
pub enum SmtpClientTraceEventPayload {
    BeginSession,
    Connected,
    Closed,
    Read(Vec<u8>),
    Command(rfc5321::Command),
    Data(Vec<u8>),
    Reply(String),
    AuthChallengeError(String),
    Diagnostic {
        level: tracing::Level,
        message: String,
    },
    MessageObtained,
}

impl SmtpClientTraceEventPayload {
    pub fn connection_error(err: &anyhow::Error) -> Self {
        let message = format!("{err:#}");
        if matches!(err.downcast_ref::<rfc5321::ClientError>(), Some(rfc5321::ClientError::Rejected(resp)) if resp.code == 334)
        {
            Self::AuthChallengeError(message)
        } else {
            Self::Diagnostic {
                level: Level::ERROR,
                message,
            }
        }
    }

    fn to_v1(self, detail: TraceDetail, terse: bool) -> Option<TraceSmtpClientV1Payload> {
        Some(match self {
            Self::BeginSession => TraceSmtpClientV1Payload::BeginSession,
            Self::Connected => TraceSmtpClientV1Payload::Connected,
            Self::MessageObtained => TraceSmtpClientV1Payload::MessageObtained,
            Self::Closed => TraceSmtpClientV1Payload::Closed,
            Self::Read(_) if detail < Secrets => return None,
            Self::Reply(_) if detail == Secrets => return None,
            Self::Read(data) => {
                TraceSmtpClientV1Payload::Read(String::from_utf8_lossy(&data).into_owned())
            }
            Self::Reply(line) => TraceSmtpClientV1Payload::Read(trace_detail::reply(line, detail)),
            Self::Command(cmd) => {
                TraceSmtpClientV1Payload::Write(trace_detail::command(&cmd, detail))
            }
            Self::Data(bytes) if detail < Content => {
                TraceSmtpClientV1Payload::Write(format!("[{} bytes]", bytes.len()))
            }
            Self::Data(bytes) => {
                let s = String::from_utf8_lossy(&bytes).into_owned();
                if terse {
                    fn split_first_line(s: &str) -> Option<String> {
                        let mut iter = s.lines();
                        let snippet = iter.next()?;
                        iter.next()?;
                        Some(snippet.to_string())
                    }
                    if let Some(snippet) = split_first_line(&s) {
                        return Some(TraceSmtpClientV1Payload::AbbreviatedWrite {
                            snippet,
                            len: bytes.len(),
                        });
                    }
                }
                TraceSmtpClientV1Payload::Write(s)
            }
            Self::AuthChallengeError(_) | Self::Diagnostic { .. } if detail < Operational => {
                return None
            }
            Self::AuthChallengeError(message) => TraceSmtpClientV1Payload::Diagnostic {
                level: Level::ERROR.to_string(),
                message: if detail == Secrets {
                    message
                } else {
                    "Unexpected SMTP 334 authentication challenge".into()
                },
            },
            Self::Diagnostic { level, message } => TraceSmtpClientV1Payload::Diagnostic {
                level: level.to_string(),
                message,
            },
        })
    }
}

fn ip_from_addr(addr: &str) -> Option<IpAddr> {
    match addr.parse::<SocketAddr>() {
        Ok(addr) => Some(addr.ip()),
        Err(_) => addr.parse::<IpAddr>().ok(),
    }
}

fn property_filters(request: &TraceSmtpClientV1Request) -> [(&'static str, &[String]); 9] {
    [
        ("campaign", &request.campaign),
        ("domain", &request.domain),
        ("egress_pool", &request.egress_pool),
        ("egress_source", &request.egress_source),
        ("ready_queue_name", &request.ready_queue),
        ("recipient", &request.rcpt_to),
        ("routing_domain", &request.routing_domain),
        ("sender", &request.mail_from),
        ("tenant", &request.tenant),
    ]
}

fn mx_matches(meta: &serde_json::Value, request: &TraceSmtpClientV1Request) -> bool {
    if request.mx_host.is_empty() && request.mx_addr.is_none() {
        return true;
    }
    let matches_peer = |host: Option<&str>, addr: Option<&str>| {
        let host_matches = request.mx_host.is_empty()
            || host.is_some_and(|host| request.mx_host.iter().any(|name| name == host));
        host_matches
            && request.mx_addr.as_ref().is_none_or(|cidrs| {
                addr.and_then(ip_from_addr)
                    .is_some_and(|ip| cidrs.contains(ip))
            })
    };
    if let Some(addr) = meta.get("mx_address") {
        return matches_peer(meta["mx_host"].as_str(), addr.as_str());
    }

    // Before the first connection, at least one planned MX must satisfy both filters.
    let plan = &meta["mx_plan"];
    if plan == "NullMx" {
        return false;
    }
    plan["Addresses"].as_array().is_none_or(|addresses| {
        addresses
            .iter()
            .any(|peer| matches_peer(peer["name"].as_str(), peer["addr"].as_str()))
    })
}

fn matches_request(meta: &serde_json::Value, request: &TraceSmtpClientV1Request) -> bool {
    if let Some(cidrs) = &request.source_addr {
        if meta["source_address"]
            .as_str()
            .and_then(ip_from_addr)
            .is_some_and(|ip| !cidrs.contains(ip))
        {
            return false;
        }
    }

    for (key, candidates) in property_filters(request) {
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
            return false;
        }
    }
    mx_matches(meta, request)
}

async fn process_websocket_inner(
    mut socket: WebSocket,
    mut rx: Receiver<SmtpClientTraceEvent>,
    subscription: Subscription,
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
    let filters = property_filters(&request);
    let connection_level = if request.source_addr.is_some()
        || request.mx_addr.is_some()
        || !request.mx_host.is_empty()
    {
        Operational
    } else {
        Protocol
    };
    let required = filters
        .iter()
        .filter(|(_, values)| !values.is_empty())
        .map(|(key, _)| trace_detail::outbound_meta_detail(key))
        .fold(connection_level, TraceDetail::max);
    if !subscription.allow_filter(required, &mut socket).await? {
        return Ok(());
    }

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

        if !matches_request(&event.conn_meta, &request) {
            continue;
        }

        let is_data = matches!(event.payload, SmtpClientTraceEventPayload::Data(_));
        let Some(event) = event.to_v1(subscription.detail, request.terse) else {
            continue;
        };
        let mut json = serde_json::to_string(&event)?;
        if is_data && json.len() > trace_detail::MAX_FRAME_SIZE {
            json = trace_detail::omitted_data(event.when, event.conn_meta)?;
        }
        socket.send(Message::Text(json.into())).await?;
    }
}

async fn process_websocket(
    socket: WebSocket,
    rx: Receiver<SmtpClientTraceEvent>,
    subscription: Subscription,
) {
    if let Err(err) = process_websocket_inner(socket, rx, subscription).await {
        tracing::error!("error in websocket: {err:#}");
    }
}

/// This is a websocket endpoint that provides outbound SMTP tracing.
/// It cannot be described via auto-generated docs extracted from the JSON Schema.
#[utoipa::path(get, tags = ["debugging", "kcli:trace-smtp-client"], path = "/api/admin/trace-smtp-client/v1")]
pub async fn trace(subscription: Subscription, ws: WebSocketUpgrade) -> impl IntoResponse {
    // Subscribe before completing the upgrade handshake. The handshake
    // response is what unblocks the connecting client, so subscribing here
    // guarantees this session is counted as a receiver before the client can
    // return and start generating traceable activity; otherwise events
    // produced in that window would be dropped by the receiver-count check in
    // SmtpClientTraceManager::submit.
    let rx = MGR.tx.subscribe();
    (
        [("kumo-trace-detail", subscription.detail.to_string())],
        ws.on_upgrade(move |socket| process_websocket(socket, rx, subscription)),
    )
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_filter_allows_setup_and_matches_the_connected_source() {
        let request = TraceSmtpClientV1Request {
            source_addr: Some(vec!["192.0.2.0/24"].try_into().unwrap()),
            ..Default::default()
        };
        let matches = |meta| matches_request(&meta, &request);
        assert!(matches(json!({})));
        assert!(matches(json!({"source_address": "192.0.2.4:54321"})));
        assert!(!matches(json!({"source_address": "198.51.100.1:54321"})));
    }

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
        assert!(!matches_request(&plan, &by_host("other.example")));
        assert!(!matches_request(&plan, &by_addr("198.51.100.0/24")));
        let mut request = by_addr("2001:db8::/32");
        request.mx_host = vec!["first.example".into()];
        assert!(!matches_request(&plan, &request)); // Host and IP match different candidates.
        request.mx_host = vec!["second.example".into()];
        assert!(matches_request(&plan, &request));
        assert!(!matches_request(&json!({"mx_plan": "NullMx"}), &request));
        assert!(!matches_request(
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
        assert!(matches_request(&meta, &request));
        meta["mx_host"] = json!("mx.example");
        meta["mx_address"] = json!("198.51.100.2");
        assert!(!matches_request(&meta, &request));
        meta["mx_address"] = json!("192.0.2.1");
        assert!(matches_request(&meta, &request));
    }

    #[test]
    fn auth_challenge_is_withheld_in_both_trace_events() {
        let rejection = |code| {
            anyhow::Error::new(rfc5321::ClientError::Rejected(
                rfc5321::Response::with_code_and_message(code, "synthetic-marker"),
            ))
            .context("authenticating to relay")
        };
        for level in [Operational, Envelope, Content, Secrets] {
            let reply = trace_detail::reply("334 synthetic-marker".into(), level);
            let error = SmtpClientTraceEventPayload::connection_error(&rejection(334));
            let Some(TraceSmtpClientV1Payload::Diagnostic { message, .. }) =
                error.to_v1(level, false)
            else {
                panic!("missing authentication diagnostic")
            };
            assert!(message.contains("334"));
            assert_eq!(message.contains("synthetic-marker"), level == Secrets);
            assert_eq!(reply.contains("synthetic-marker"), level == Secrets);
        }
        let ordinary = rejection(535);
        let Some(TraceSmtpClientV1Payload::Diagnostic { message, .. }) =
            SmtpClientTraceEventPayload::connection_error(&ordinary).to_v1(Operational, false)
        else {
            panic!("missing rejection diagnostic")
        };
        assert_eq!(message, format!("{ordinary:#}"));
    }

    #[test]
    fn ready_queue_filters_setup_events_before_recipients_are_known() {
        let request = TraceSmtpClientV1Request {
            ready_queue: vec!["wanted".into()],
            rcpt_to: vec!["first@example.test".into()],
            ..Default::default()
        };
        let matches = |name| matches_request(&json!({"ready_queue_name": name}), &request);
        assert!(matches("wanted"));
        assert!(!matches("other"));
    }

    #[test]
    fn optional_message_fields_must_match_once_a_message_is_selected() {
        let request = TraceSmtpClientV1Request {
            campaign: vec!["campaign".into()],
            tenant: vec!["tenant".into()],
            routing_domain: vec!["relay.example".into()],
            ..Default::default()
        };
        assert!(matches_request(&json!({}), &request));
        let message = json!({
            "message_id": "id", "campaign": "campaign", "tenant": "tenant", "routing_domain": "relay.example"
        });
        assert!(matches_request(&message, &request));
        for field in ["campaign", "tenant", "routing_domain"] {
            let mut missing = message.clone();
            if field == "routing_domain" {
                missing[field] = serde_json::Value::Null;
            } else {
                missing.as_object_mut().unwrap().remove(field);
            }
            assert!(!matches_request(&missing, &request), "missing {field}");
        }
    }

    #[test]
    fn rcpt_to_matches_any_message_recipient() {
        let request = TraceSmtpClientV1Request {
            rcpt_to: vec!["first@example.test".into(), "second@example.test".into()],
            ..Default::default()
        };
        let matches =
            |recipients: &[&str]| matches_request(&json!({"recipient": recipients}), &request);
        assert!(matches(&["other@example.test", "second@example.test"]));
        assert!(!matches(&["other@example.test"]));
        assert!(!matches(&[]));
    }
}
