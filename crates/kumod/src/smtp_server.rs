use crate::delivery_metrics::MetricsWrappedConnection;
use crate::http_server::admin_trace_smtp_server_v1::{
    SmtpServerTraceEvent, SmtpServerTraceEventPayload, SmtpServerTraceManager,
};
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::logging::rejection::{log_rejection, LogRejection};
use crate::queue::{DeliveryProto, IncrementAttempts, QueueConfig, QueueManager};
use crate::ready_queue::{Dispatcher, QueueDispatcher};
use crate::spool::SpoolManager;
use anyhow::{anyhow, Context};
use async_trait::async_trait;
use chrono::Utc;
use cidr_map::CidrSet;
use config::{any_err, load_config, serialize_options, CallbackSignature};
use data_encoding::BASE64;
use data_loader::KeySource;
use kumo_log_types::ResolvedAddress;
use kumo_prometheus::AtomicCounter;
use kumo_server_lifecycle::{Activity, ShutdownSubcription};
use kumo_server_runtime::{spawn, Runtime};
use mailparsing::ConformanceDisposition;
use memchr::memmem::Finder;
use message::{EnvelopeAddress, Message};
use mlua::prelude::LuaUserData;
use mlua::{FromLuaMulti, IntoLuaMulti, LuaSerdeExt, UserData, UserDataMethods};
use parking_lot::FairMutex as Mutex;
use prometheus::{Histogram, HistogramTimer};
use rfc5321::{AsyncReadAndWrite, BoxedAsyncReadAndWrite, Command, Response};
use rustls::ServerConfig;
use serde::{Deserialize, Serialize};
use serde_json::json;
use spool::SpoolId;
use std::collections::HashMap;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, instrument, Level};
use utoipa::ToSchema;
use uuid::Uuid;

pub const DEFERRED_QUEUE_NAME: &str = "deferred_smtp_inject.kumomta.internal";

static SMTP_SERVER_MSG_RX: LazyLock<CallbackSignature<(Message, ConnectionMetaData), ()>> =
    LazyLock::new(|| CallbackSignature::new("smtp_server_message_received"));

static DEFERRED_SMTP_SERVER_MSG_INJECT: LazyLock<
    CallbackSignature<(Message, ConnectionMetaData), ()>,
> = LazyLock::new(|| CallbackSignature::new("smtp_server_message_deferred_inject"));

static CRLF: LazyLock<Finder> = LazyLock::new(|| Finder::new("\r\n"));
static TXN_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "smtpsrv_transaction_duration",
        "how long an incoming SMTP transaction takes",
    )
    .unwrap()
});
static READ_DATA_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "smtpsrv_read_data_duration",
        "how long it takes to receive the DATA portion",
    )
    .unwrap()
});
static PROCESS_DATA_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "smtpsrv_process_data_duration",
        "how long it takes to process the DATA portion and enqueue",
    )
    .unwrap()
});

#[derive(Debug, Hash, PartialEq, Eq)]
struct DomainAndListener {
    pub domain: String,
    pub listener: String,
}

static SMTPSRV: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new("smtpsrv", |cpus| cpus * 3 / 8, &SMTPSRV_THREADS).unwrap());

static SMTPSRV_THREADS: AtomicUsize = AtomicUsize::new(0);

pub fn set_smtpsrv_threads(n: usize) {
    SMTPSRV_THREADS.store(n, Ordering::SeqCst);
}

#[derive(Deserialize, Clone, Debug, Default, Serialize, mlua::FromLua)]
#[serde(deny_unknown_fields)]
pub struct EsmtpDomain {
    #[serde(default)]
    pub log_oob: bool,
    #[serde(default)]
    pub log_arf: bool,
    #[serde(default)]
    pub relay_to: bool,
    #[serde(default)]
    pub relay_from: CidrSet,

    // Deprecated and no longer used
    #[serde(default = "default_ttl", with = "duration_serde")]
    pub ttl: Duration,
}

impl LuaUserData for EsmtpDomain {}

fn default_ttl() -> Duration {
    Duration::from_secs(60)
}

#[derive(Deserialize, Serialize, Clone, Debug, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceHeaders {
    /// Whether to add a Received: header
    #[serde(default = "default_true")]
    pub received_header: bool,

    /// Whether to add a supplemental trace header to encode
    /// additional metadata
    #[serde(default = "default_true")]
    pub supplemental_header: bool,

    /// The name of the supplemental trace header
    #[serde(default = "TraceHeaders::default_header_name")]
    pub header_name: String,

    /// List of meta keys that should be included in the
    /// supplemental header
    #[serde(default = "TraceHeaders::default_meta")]
    pub include_meta_names: Vec<String>,
}

impl Default for TraceHeaders {
    fn default() -> Self {
        Self {
            received_header: true,
            supplemental_header: true,
            header_name: Self::default_header_name(),
            include_meta_names: vec![],
        }
    }
}

impl TraceHeaders {
    fn default_header_name() -> String {
        "X-KumoRef".to_string()
    }

    fn default_meta() -> Vec<String> {
        vec![]
    }

    pub fn apply_supplemental(&self, message: &Message) -> anyhow::Result<()> {
        if !self.supplemental_header {
            return Ok(());
        }
        let mut object = json!({
            // Marker to identify encoded supplemental header
            "_@_": "\\_/",
            "recipient": message.recipient()?,
        });

        for name in &self.include_meta_names {
            if let Ok(value) = message.get_meta(name) {
                object
                    .as_object_mut()
                    .unwrap()
                    .insert(name.to_string(), value);
            }
        }

        let value = BASE64.encode(serde_json::to_string(&object)?.as_bytes());
        message.prepend_header(Some(&self.header_name), &value);

        Ok(())
    }
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct EsmtpListenerParams {
    #[serde(default = "EsmtpListenerParams::default_listen")]
    pub listen: String,
    #[serde(default = "EsmtpListenerParams::default_hostname")]
    pub hostname: String,
    #[serde(default = "CidrSet::default_trusted_hosts")]
    pub relay_hosts: CidrSet,
    #[serde(default = "EsmtpListenerParams::default_banner")]
    pub banner: String,

    #[serde(default)]
    pub tls_certificate: Option<KeySource>,
    #[serde(default)]
    pub tls_private_key: Option<KeySource>,

    #[serde(default)]
    pub deferred_spool: bool,

    #[serde(default)]
    pub deferred_queue: bool,

    #[serde(default)]
    pub trace_headers: TraceHeaders,

    #[serde(
        default = "EsmtpListenerParams::default_client_timeout",
        with = "duration_serde"
    )]
    pub client_timeout: Duration,

    #[serde(skip)]
    tls_config: OnceLock<Arc<ServerConfig>>,

    #[serde(skip)]
    connection_gauge: OnceLock<AtomicCounter>,

    #[serde(skip)]
    connection_denied_counter: OnceLock<AtomicCounter>,

    #[serde(default = "EsmtpListenerParams::default_max_messages_per_connection")]
    max_messages_per_connection: usize,
    #[serde(default = "EsmtpListenerParams::default_max_recipients_per_message")]
    max_recipients_per_message: usize,

    #[serde(default = "EsmtpListenerParams::default_max_message_size")]
    max_message_size: usize,

    #[serde(default = "EsmtpListenerParams::default_max_connections")]
    max_connections: usize,

    #[serde(default = "EsmtpListenerParams::default_data_buffer_size")]
    data_buffer_size: usize,

    #[serde(default)]
    invalid_line_endings: ConformanceDisposition,

    #[serde(default = "EsmtpListenerParams::default_line_length_hard_limit")]
    line_length_hard_limit: usize,
}

impl EsmtpListenerParams {
    fn default_max_messages_per_connection() -> usize {
        10_000
    }

    fn default_max_recipients_per_message() -> usize {
        1024
    }

    fn default_max_message_size() -> usize {
        20 * 1024 * 1024
    }

    fn default_max_connections() -> usize {
        32 * 1024
    }

    fn default_data_buffer_size() -> usize {
        128 * 1024
    }

    fn default_client_timeout() -> Duration {
        Duration::from_secs(60)
    }

    fn default_line_length_hard_limit() -> usize {
        MAX_LINE_LEN
    }

    fn default_listen() -> String {
        "127.0.0.1:2025".to_string()
    }

    pub fn default_hostname() -> String {
        gethostname::gethostname()
            .to_str()
            .unwrap_or("localhost")
            .to_string()
    }

    fn default_banner() -> String {
        "KumoMTA".to_string()
    }

    pub async fn build_tls_acceptor(&self) -> anyhow::Result<TlsAcceptor> {
        if let Some(config) = self.tls_config.get() {
            return Ok(TlsAcceptor::from(config.clone()));
        }

        let config = kumo_server_common::tls_helpers::make_server_config(
            &self.hostname,
            &self.tls_private_key,
            &self.tls_certificate,
        )
        .await?;

        // If we race to create, take the winner's version
        Ok(TlsAcceptor::from(
            self.tls_config.get_or_init(|| config).clone(),
        ))
    }

    pub fn connection_gauge(&self) -> &AtomicCounter {
        self.connection_gauge
            .get_or_init(|| crate::metrics_helper::connection_gauge_for_service("esmtp_listener"))
    }

    pub fn connection_denied_counter(&self) -> &AtomicCounter {
        self.connection_denied_counter
            .get_or_init(|| crate::metrics_helper::connection_denied_for_service("esmtp_listener"))
    }

    pub async fn run(self) -> anyhow::Result<()> {
        // Pre-create the acceptor so that we can share it across
        // the various listeners
        self.build_tls_acceptor().await?;
        self.connection_gauge();

        let listener = TcpListener::bind(&self.listen)
            .await
            .with_context(|| format!("failed to bind to {}", self.listen))?;

        let addr = listener.local_addr()?;
        tracing::info!("smtp listener on {addr:?}");

        let mut shutting_down = ShutdownSubcription::get();
        let connection_limiter = Arc::new(tokio::sync::Semaphore::new(self.max_connections));
        spawn(format!("esmtp_listener {addr:?}"), async move {
            let denied = self.connection_denied_counter();
            loop {
                tokio::select! {
                    _ = shutting_down.shutting_down() => {
                        tracing::info!("smtp listener on {addr:?} -> stopping");
                        return Ok::<(), anyhow::Error>(());
                    }
                    result = listener.accept() => {
                        let (mut socket, peer_address) = result?;
                        let Ok(permit) = connection_limiter.clone().try_acquire_owned() else {
                            // We're over the limit. We make a "best effort" to respond;
                            // don't strain too hard here, as the purpose of the limit is
                            // to constrain resource utilization, so no sense going too
                            // hard in this case.

                            // Bump the connection denied counter, because the operator
                            // may want to note that we're at the limit and do something
                            // to mitigate it.
                            denied.inc();

                            let hostname = &self.hostname;
                            let response = format!("421 4.3.2 {hostname} too many concurrent sessions. Try later\r\n");
                            // We allow up to 2 seconds to write the response to
                            // the peer. Since we're not spawning this task, further
                            // accepts are blocked for up to that duration.
                            // That is OK as we're over our limit on connections
                            // anyway and don't want to/can't accept new connections
                            // right now anyway.
                            // We want to avoid spawning because that would allocate
                            // more memory and introduce additional concerns around
                            // tracking additional connections in the metrics.
                            // This way we should never have more than N+1 incoming
                            // connections on this listener.
                            let _ = tokio::time::timeout(
                                Duration::from_secs(2),
                                socket.write(response.as_bytes())
                            ).await;
                            drop(socket);
                            continue;
                        };

                        // No need for Nagle with SMTP request/response
                        socket.set_nodelay(true)?;
                        let my_address = socket.local_addr()?;
                        let params = self.clone();
                        SMTPSRV.spawn(
                            format!("SmtpServer {peer_address:?}"),
                            async move {
                                if let Err(err) =
                                    SmtpServer::run(socket, my_address, peer_address, params).await
                                    {
                                        tracing::error!("SmtpServer::run: {err:#}");
                                }
                                drop(permit);
                            }
                        )?;
                    }
                };
            }
        })?;
        Ok(())
    }
}

#[derive(Error, Debug, Clone)]
#[error("{code} {message}")]
#[must_use]
pub struct RejectError {
    /// SMTP 3-digit response code
    pub code: u16,
    /// The textual portion of the response to send
    pub message: String,
}

impl RejectError {
    pub fn from_lua(err: &mlua::Error) -> Option<Self> {
        match err {
            mlua::Error::CallbackError { cause, .. } => return Self::from_lua(cause),
            mlua::Error::ExternalError(err) => return Self::from_std_error(&**err),
            _ => None,
        }
    }

    pub fn from_std_error(err: &(dyn std::error::Error + 'static)) -> Option<Self> {
        if let Some(cause) = err.source() {
            return Self::from_std_error(cause);
        } else if let Some(rej) = err.downcast_ref::<Self>() {
            Some(rej.clone())
        } else if let Some(lua_err) = err.downcast_ref::<mlua::Error>() {
            Self::from_lua(lua_err)
        } else {
            None
        }
    }

    pub fn from_anyhow(err: &anyhow::Error) -> Option<Self> {
        Self::from_std_error(err.root_cause())
    }
}

/// Helper for tracing/printing as human readable text rather than
/// an array of decimal numbers
struct DebugPrintBuffer<'a>(&'a [u8]);

impl<'a> std::fmt::Debug for DebugPrintBuffer<'a> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = String::from_utf8_lossy(&self.0);
        write!(fmt, "{} bytes: {s:?}", self.0.len())
    }
}

struct DebugabbleReadBuffer(Vec<u8>);

impl std::ops::Deref for DebugabbleReadBuffer {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for DebugabbleReadBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl std::fmt::Debug for DebugabbleReadBuffer {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "DebugabbleReadBuffer({:?})", DebugPrintBuffer(&self.0))
    }
}

pub struct SmtpServer {
    socket: Option<BoxedAsyncReadAndWrite>,
    state: Option<TransactionState>,
    said_hello: Option<String>,
    peer_address: SocketAddr,
    my_address: SocketAddr,
    tls_active: bool,
    read_buffer: DebugabbleReadBuffer,
    params: EsmtpListenerParams,
    shutdown: ShutdownSubcription,
    rcpt_count: usize,
    authorization_id: Option<String>,
    authentication_id: Option<String>,
    meta: ConnectionMetaData,
    global_reception_count: AtomicCounter,
    reception_count: AtomicCounter,
    session_id: Uuid,
    domains: HashMap<String, Option<EsmtpDomain>>,
}

#[derive(Debug)]
struct TransactionState {
    sender: EnvelopeAddress,
    recipients: Vec<EnvelopeAddress>,
    _timer: HistogramTimer,
}

#[derive(Copy, Clone, Debug)]
pub struct RelayDisposition {
    /// Should queue for onward delivery
    pub relay: bool,
    /// Should accept to process ARF reports
    pub log_arf: bool,
    pub log_oob: bool,
}

impl RelayDisposition {
    pub fn accept_rcpt_to(&self) -> bool {
        self.relay || self.log_arf || self.log_oob
    }
}

impl SmtpServer {
    #[instrument(skip(params, my_address, peer_address))]
    pub async fn run<T>(
        socket: T,
        my_address: SocketAddr,
        peer_address: SocketAddr,
        params: EsmtpListenerParams,
    ) -> anyhow::Result<()>
    where
        T: AsyncReadAndWrite + Debug + Send + 'static,
    {
        let socket: BoxedAsyncReadAndWrite = Box::new(socket);

        let mut meta = ConnectionMetaData::new();
        meta.set_meta("reception_protocol", "ESMTP");
        meta.set_meta("received_via", my_address.to_string());
        meta.set_meta("received_from", peer_address.to_string());
        meta.set_meta("hostname", params.hostname.to_string());

        let service = format!("esmtp_listener:{my_address}");

        let mut server = SmtpServer {
            socket: Some(socket),
            state: None,
            said_hello: None,
            peer_address,
            my_address,
            tls_active: false,
            read_buffer: DebugabbleReadBuffer(Vec::with_capacity(1024)),
            params,
            shutdown: ShutdownSubcription::get(),
            rcpt_count: 0,
            authorization_id: None,
            authentication_id: None,
            meta,
            reception_count: crate::metrics_helper::total_msgs_received_for_service(&service),
            global_reception_count: crate::metrics_helper::total_msgs_received_for_service(
                "esmtp_listener",
            ),
            session_id: Uuid::new_v4(),
            domains: HashMap::new(),
        };

        server.params.connection_gauge().inc();

        SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
            conn_meta: server.meta.clone_inner(),
            payload: SmtpServerTraceEventPayload::Connected,
            when: Utc::now(),
        });

        if let Err(err) = server.process().await {
            if err.downcast_ref::<WriteError>().is_none() {
                error!("Error in SmtpServer: {err:#}");
                server
                    .write_response(
                        421,
                        format!("4.3.0 {} technical difficulties", server.params.hostname),
                        Some(format!("Error in SmtpServer: {err:#}")),
                    )
                    .await
                    .ok();
            }
        }
        server.params.connection_gauge().dec();

        SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
            conn_meta: server.meta.clone_inner(),
            payload: SmtpServerTraceEventPayload::Closed,
            when: Utc::now(),
        });

        Ok(())
    }

    fn peer_in_cidr_list(&self, cidr: &CidrSet) -> bool {
        cidr.contains(self.peer_address.ip())
    }

    async fn lookup_listener_domain(
        &mut self,
        domain_name: &str,
    ) -> anyhow::Result<Option<EsmtpDomain>> {
        let key = DomainAndListener {
            domain: domain_name.to_string(),
            listener: self.my_address.to_string(),
        };

        if let Some(opt_dom) = self.domains.get(domain_name) {
            return Ok(opt_dom.clone());
        }

        let mut config = load_config().await?;

        let sig =
            CallbackSignature::<(String, String, ConnectionMetaData), Option<EsmtpDomain>>::new(
                "get_listener_domain",
            );
        let value: anyhow::Result<Option<EsmtpDomain>> = config
            .async_call_callback_non_default_opt(
                &sig,
                (key.domain.clone(), key.listener.clone(), self.meta.clone()),
            )
            .await;

        let value = match value {
            Ok(v) => {
                SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                    conn_meta: self.meta.clone_inner(),
                    payload: SmtpServerTraceEventPayload::Callback {
                        name: format!("get_listener_domain domain={domain_name}"),
                        result: serde_json::to_value(&v).ok(),
                        error: None,
                    },
                    when: Utc::now(),
                });
                v
            }
            Err(err) => {
                SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                    conn_meta: self.meta.clone_inner(),
                    payload: SmtpServerTraceEventPayload::Callback {
                        name: format!("get_listener_domain domain={domain_name}"),
                        result: None,
                        error: Some(format!("{err:#}")),
                    },
                    when: Utc::now(),
                });
                return Err(err);
            }
        };

        // Remember a bounded number of entries, so that an abusive
        // client can't trivially use up a lot of ram by trying a
        // lot of random domains
        while self.domains.len() > 16 {
            let key = self
                .domains
                .keys()
                .next()
                .expect("have at least one key when !empty")
                .to_string();
            self.domains.remove(&key);
        }
        self.domains.insert(domain_name.to_string(), value.clone());

        Ok(value)
    }

    async fn check_relaying(
        &mut self,
        sender: &EnvelopeAddress,
        recipient: &EnvelopeAddress,
    ) -> anyhow::Result<RelayDisposition> {
        let relay_hosts_allowed = self.peer_in_cidr_list(&self.params.relay_hosts);

        let sender_domain = sender.domain();
        let mut relay_from_allowed = false;

        if let Some(dom) = self.lookup_listener_domain(&sender_domain).await? {
            relay_from_allowed = self.peer_in_cidr_list(&dom.relay_from);
        }

        let recipient_domain = recipient.domain();
        let mut relay_to_allowed = None;
        let mut log_arf = false;
        let mut log_oob = false;

        if let Some(dom) = self.lookup_listener_domain(&recipient_domain).await? {
            relay_to_allowed.replace(dom.relay_to);
            log_arf = dom.log_arf;
            log_oob = dom.log_oob;
        }

        // Check the rules for relaying-from first; that allows
        // things like CIDR or explicit SMTP authentication to
        // take effect for a sender before we consider a "random"
        // destination domain for which relay_to will likely be
        // set to false.
        let relay = if relay_hosts_allowed || relay_from_allowed || relay_to_allowed == Some(true) {
            true
        } else {
            false
        };

        tracing::debug!(
            "check_relaying: sender={sender_domain} \
             recip={recipient_domain} relay_to_allowed={relay_to_allowed:?} \
             relay_hosts_allowed={relay_hosts_allowed} \
             relay_from_allowed={relay_from_allowed} \
             -> log_arf={log_arf} log_oob={log_oob} relay={relay}"
        );

        Ok(RelayDisposition {
            relay,
            log_arf,
            log_oob,
        })
    }

    #[instrument(skip(self))]
    async fn write_response<S: AsRef<str> + Debug>(
        &mut self,
        status: u16,
        message: S,
        command: Option<String>,
    ) -> Result<(), WriteError> {
        if let Some(socket) = self.socket.as_mut() {
            if status >= 400
                && status < 600
                // Don't log the shutting down message, or load shedding messages.
                // The main purpose of Rejection logging is to see what unexpected and
                // unsuccessful results are being returned to the peer.
                // If we log rejections via log hooks during a memory shortage,
                // we're increasing our memory burden instead of avoiding it.
                && !(status == 421 && message.as_ref().starts_with("4.3.2 "))
            {
                let mut response = Response::with_code_and_message(status, message.as_ref());
                response.command = command;

                let mut sender = None;
                let mut recipient = None;
                if let Some(state) = &self.state {
                    sender.replace(state.sender.to_string());
                    recipient = state.recipients.last().map(|r| r.to_string());
                }

                log_rejection(LogRejection {
                    meta: self.meta.clone_inner(),
                    peer_address: ResolvedAddress {
                        name: self.said_hello.as_deref().unwrap_or("").to_string(),
                        addr: self.peer_address.ip().into(),
                    },
                    response,
                    sender,
                    recipient,
                    session_id: Some(self.session_id),
                })
                .await;
            }

            let mut lines = message.as_ref().lines().peekable();
            while let Some(line) = lines.next() {
                let is_last = lines.peek().is_none();
                let sep = if is_last { ' ' } else { '-' };
                let text = format!("{status}{sep}{line}\r\n");

                SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                    conn_meta: self.meta.clone_inner(),
                    payload: SmtpServerTraceEventPayload::Write(text.clone()),
                    when: Utc::now(),
                });

                tracing::trace!("writing response line: {text}");
                socket
                    .write(text.as_bytes())
                    .await
                    .map_err(|_| WriteError {})?;
            }
            socket.flush().await.map_err(|_| WriteError {})?;

            if status == 421 {
                // 421 is only valid when disconnecting the session,
                // so disconnect it!
                self.socket.take();
            }
        }
        Ok(())
    }

    fn check_shutdown(&self) -> bool {
        if self.read_buffer.is_empty() {
            Activity::get_opt(format!("SMTP server check_shutdown (transient)")).is_none()
        } else {
            false
        }
    }

    #[instrument(skip(self))]
    async fn read_data(&mut self) -> anyhow::Result<ReadData> {
        let mut too_big = false;
        tracing::trace!("reading data");

        static CRLFDOTCRLF: LazyLock<Finder> = LazyLock::new(|| Finder::new("\r\n.\r\n"));
        let mut data = DebugabbleReadBuffer(vec![0u8; self.params.data_buffer_size]);
        let mut next_index = 0;

        loop {
            if let Some(i) = CRLFDOTCRLF.find(&self.read_buffer[next_index..]) {
                let i = i + next_index;

                if too_big {
                    self.read_buffer.drain(0..i + 5);
                    SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                        conn_meta: self.meta.clone_inner(),
                        payload: SmtpServerTraceEventPayload::Diagnostic {
                            level: Level::ERROR,
                            message: "Data too big".to_string(),
                        },
                        when: Utc::now(),
                    });
                    return Ok(ReadData::TooBig);
                }

                let mut tail = self.read_buffer.split_off(i + 2);
                std::mem::swap(&mut tail, &mut self.read_buffer);
                self.read_buffer.drain(0..3);

                let data = unstuff(tail);

                if !check_line_lengths(&data, self.params.line_length_hard_limit) {
                    SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                        conn_meta: self.meta.clone_inner(),
                        payload: SmtpServerTraceEventPayload::Diagnostic {
                            level: Level::ERROR,
                            message: "Line too long".to_string(),
                        },
                        when: Utc::now(),
                    });
                    return Ok(ReadData::TooLong);
                }

                tracing::trace!("returning ReadData::Data {:?}", DebugPrintBuffer(&data));
                return Ok(ReadData::Data(data));
            }

            tracing::trace!("read_buffer len is {}", self.read_buffer.len());
            let buf_len = self.read_buffer.len();
            next_index = buf_len.saturating_sub(5);
            if buf_len >= self.params.max_message_size {
                too_big = true;
                self.read_buffer.drain(0..next_index);
                next_index = 0;
            }

            // Didn't find terminator, fill up the buffer
            tokio::select! {
                _ = tokio::time::sleep(self.params.client_timeout) => {
                    return Ok(ReadData::TimedOut);
                }
                size = self.socket.as_mut().unwrap().read(&mut data) => {
                    match size {
                        Err(err) => {
                            tracing::trace!("error reading: {err:#}");
                            SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                                conn_meta: self.meta.clone_inner(),
                                payload: SmtpServerTraceEventPayload::Diagnostic {
                                    level: Level::ERROR,
                                    message: format!("error reading: {err:#}"),
                                },
                                when: Utc::now(),
                            });
                            return Ok(ReadData::Disconnected);
                        }
                        Ok(size) if size == 0 => {
                            SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                                conn_meta: self.meta.clone_inner(),
                                payload: SmtpServerTraceEventPayload::Diagnostic {
                                    level: Level::ERROR,
                                    message: "Peer Disconnected".to_string(),
                                },
                                when: Utc::now(),
                            });
                            return Ok(ReadData::Disconnected);
                        }
                        Ok(size) => {
                            SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                                conn_meta: self.meta.clone_inner(),
                                payload: SmtpServerTraceEventPayload::Read(data[0..size].to_vec()),
                                when: Utc::now(),
                            });
                            self.read_buffer.extend_from_slice(&data[0..size]);
                        }
                    }
                }
                _ = self.shutdown.shutting_down() => {
                    return Ok(ReadData::ShuttingDown);
                }
            };
        }
    }

    #[instrument(skip(self))]
    async fn read_line(&mut self, override_limit: Option<usize>) -> anyhow::Result<ReadLine> {
        if self.socket.is_none() {
            return Ok(ReadLine::Disconnected);
        }

        let mut too_long = false;
        tracing::trace!("reading line");

        loop {
            if let Some(i) = CRLF.find(&self.read_buffer) {
                if too_long {
                    self.read_buffer.drain(0..i + 2);
                    SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                        conn_meta: self.meta.clone_inner(),
                        payload: SmtpServerTraceEventPayload::Diagnostic {
                            level: Level::ERROR,
                            message: "Line too long".to_string(),
                        },
                        when: Utc::now(),
                    });
                    return Ok(ReadLine::TooLong);
                }

                let line = String::from_utf8(self.read_buffer[0..i].to_vec());

                self.read_buffer.drain(0..i + 2);
                tracing::trace!("{line:?}");
                return Ok(ReadLine::Line(line?));
            }
            tracing::trace!("read_buffer len is {}", self.read_buffer.len());
            if self.read_buffer.len() > override_limit.unwrap_or(self.params.line_length_hard_limit)
            {
                self.read_buffer.clear();
                too_long = true;
            }

            // Didn't find a complete line, fill up the rest of the buffer
            let mut data = [0u8; 1024];
            tokio::select! {
                _ = tokio::time::sleep(self.params.client_timeout) => {
                    return Ok(ReadLine::TimedOut);
                }
                size = self.socket.as_mut().unwrap().read(&mut data) => {
                    match size {
                        Err(err) => {
                            tracing::trace!("error reading: {err:#}");
                            SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                                conn_meta: self.meta.clone_inner(),
                                payload: SmtpServerTraceEventPayload::Diagnostic {
                                    level: Level::ERROR,
                                    message: format!("error reading: {err:#}"),
                                },
                                when: Utc::now(),
                            });
                            return Ok(ReadLine::Disconnected);
                        }
                        Ok(size) if size == 0 => {
                            SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                                conn_meta: self.meta.clone_inner(),
                                payload: SmtpServerTraceEventPayload::Diagnostic {
                                    level: Level::ERROR,
                                    message: "Peer Disconnected".to_string(),
                                },
                                when: Utc::now(),
                            });
                            return Ok(ReadLine::Disconnected);
                        }
                        Ok(size) => {
                            SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                                conn_meta: self.meta.clone_inner(),
                                payload: SmtpServerTraceEventPayload::Read(data[0..size].to_vec()),
                                when: Utc::now(),
                            });
                            self.read_buffer.extend_from_slice(&data[0..size]);
                        }
                    }
                }
                _ = self.shutdown.shutting_down() => {
                    return Ok(ReadLine::ShuttingDown);
                }
            };
        }
    }

    async fn call_callback_sig<
        R: FromLuaMulti + Default + serde::Serialize,
        A: IntoLuaMulti + Clone,
    >(
        &mut self,
        sig: &CallbackSignature<A, R>,
        args: A,
    ) -> anyhow::Result<Result<R, RejectError>> {
        let mut config = load_config().await?;
        let name = sig.name();
        match config.async_call_callback(sig, args).await {
            Ok(r) => {
                SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                    conn_meta: self.meta.clone_inner(),
                    payload: SmtpServerTraceEventPayload::Callback {
                        name: name.to_string(),
                        result: serde_json::to_value(&r).ok(),
                        error: None,
                    },
                    when: Utc::now(),
                });

                Ok(Ok(r))
            }
            Err(err) => {
                SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                    conn_meta: self.meta.clone_inner(),
                    payload: SmtpServerTraceEventPayload::Callback {
                        name: name.to_string(),
                        result: None,
                        error: Some(format!("{err:#}")),
                    },
                    when: Utc::now(),
                });
                if let Some(rej) = RejectError::from_anyhow(&err) {
                    Ok(Err(rej))
                } else {
                    Err(err)
                }
            }
        }
    }

    pub async fn call_callback<
        R: FromLuaMulti + Default + serde::Serialize,
        S: Into<std::borrow::Cow<'static, str>>,
        A: IntoLuaMulti + Clone,
    >(
        &mut self,
        name: S,
        args: A,
    ) -> anyhow::Result<Result<R, RejectError>> {
        let name = name.into();
        let sig = CallbackSignature::<A, R>::new(name);
        self.call_callback_sig(&sig, args).await
    }

    #[instrument(skip(self))]
    async fn process(&mut self) -> anyhow::Result<()> {
        let _activity = match Activity::get_opt(format!(
            "smtp_server process client {:?} -> {:?}",
            self.peer_address, self.my_address
        )) {
            None => {
                // We don't bump the connection_denied_counter here, because
                // shutdown is not (on its own) a condition that requires
                // alerting and response.

                // Can't accept any messages while we're shutting down
                self.write_response(
                    421,
                    format!("4.3.2 {} shutting down", self.params.hostname),
                    None,
                )
                .await?;
                return Ok(());
            }
            Some(a) => a,
        };
        if kumo_server_memory::get_headroom() == 0 {
            // Bump connection_denied_counter because the operator may care to
            // investigate this, and we don't otherwise log this class of rejection.
            self.params.connection_denied_counter().inc();

            // Using too much memory
            self.write_response(
                421,
                format!("4.3.2 {} load shedding. Try later", self.params.hostname),
                None,
            )
            .await?;
            return Ok(());
        }
        if kumo_server_common::disk_space::is_over_limit() {
            // Bump connection_denied_counter because the operator may care to
            // investigate this, and we don't otherwise log this class of rejection.
            self.params.connection_denied_counter().inc();

            self.write_response(
                421,
                format!("4.3.2 {} disk is too full. Try later", self.params.hostname),
                None,
            )
            .await?;
            return Ok(());
        }

        if !SpoolManager::get().spool_started() {
            // We don't bump the connection_denied_counter here, because
            // startup is a normal condition and doesn't require an operator
            // to respond.

            // Can't accept any messages until the spool has started
            // because we won't know where to put them.
            self.write_response(
                421,
                format!(
                    "4.3.2 {} waiting for spool startup. Try again soon!",
                    self.params.hostname
                ),
                None,
            )
            .await?;
            return Ok(());
        }

        self.write_response(
            220,
            format!("{} {}", self.params.hostname, self.params.banner),
            None,
        )
        .await?;
        loop {
            if self.check_shutdown() {
                self.write_response(
                    421,
                    format!("4.3.2 {} shutting down", self.params.hostname),
                    None,
                )
                .await?;
                return Ok(());
            }

            let line = match self.read_line(None).await? {
                ReadLine::Disconnected => return Ok(()),
                ReadLine::Line(line) => line,
                ReadLine::TimedOut => {
                    self.write_response(
                        421,
                        format!("4.3.2 {} idle too long", self.params.hostname),
                        None,
                    )
                    .await?;
                    return Ok(());
                }
                ReadLine::ShuttingDown => {
                    self.write_response(
                        421,
                        format!("4.3.2 {} shutting down", self.params.hostname),
                        None,
                    )
                    .await?;
                    return Ok(());
                }
                ReadLine::TooLong => {
                    self.write_response(500, "5.2.3 line too long", None)
                        .await?;
                    continue;
                }
            };

            match Command::parse(&line) {
                Err(err) => {
                    self.write_response(
                        501,
                        format!("Syntax error in command or arguments: {err}"),
                        Some(line),
                    )
                    .await?;
                }
                Ok(Command::Quit) => {
                    self.write_response(221, "So long, and thanks for all the fish!", None)
                        .await?;
                    return Ok(());
                }
                Ok(Command::StartTls) => {
                    if self.tls_active {
                        self.write_response(
                            501,
                            "Cannot STARTTLS as TLS is already active",
                            Some(line),
                        )
                        .await?;
                        continue;
                    }
                    self.write_response(220, "Ready to Start TLS", None).await?;
                    let acceptor = self.params.build_tls_acceptor().await?;
                    let socket: BoxedAsyncReadAndWrite = match acceptor
                        .accept(self.socket.take().unwrap())
                        .into_fallible()
                        .await
                    {
                        Ok(stream) => {
                            self.tls_active = true;
                            Box::new(stream)
                        }
                        Err((err, stream)) => {
                            tracing::debug!("TLS handshake failed: {err:#}");
                            stream
                        }
                    };
                    self.socket.replace(socket);
                }
                Ok(Command::Auth {
                    sasl_mech,
                    initial_response,
                }) => {
                    if self.authentication_id.is_some() {
                        self.write_response(
                            503,
                            "5.5.1 AUTH me once, can't get authed again!",
                            Some(line),
                        )
                        .await?;
                        continue;
                    }
                    if self.state.is_some() {
                        self.write_response(
                            503,
                            "5.5.1 AUTH not permitted inside a transaction",
                            Some(line),
                        )
                        .await?;
                        continue;
                    }
                    if sasl_mech != "PLAIN" {
                        self.write_response(
                            504,
                            format!("5.5.4 AUTH {sasl_mech} not supported"),
                            Some(line),
                        )
                        .await?;
                        continue;
                    }
                    if !self.tls_active {
                        self.write_response(
                            524,
                            format!("5.7.11 AUTH {sasl_mech} requires an encrypted channel"),
                            Some(line),
                        )
                        .await?;
                        continue;
                    }

                    let response = if let Some(r) = initial_response {
                        r
                    } else {
                        self.write_response(334, " ", None).await?;
                        match self.read_line(Some(16384)).await? {
                            ReadLine::Disconnected => return Ok(()),
                            ReadLine::Line(line) => line,
                            ReadLine::TimedOut => {
                                self.write_response(
                                    421,
                                    format!("4.3.2 {} idle too long", self.params.hostname),
                                    Some(line),
                                )
                                .await?;
                                return Ok(());
                            }
                            ReadLine::ShuttingDown => {
                                self.write_response(
                                    421,
                                    format!("4.3.2 {} shutting down", self.params.hostname),
                                    Some(line),
                                )
                                .await?;
                                return Ok(());
                            }
                            ReadLine::TooLong => {
                                self.write_response(
                                    500,
                                    "5.5.6 authentication exchange line too long",
                                    Some(line),
                                )
                                .await?;
                                continue;
                            }
                        }
                    };

                    if response == "*" {
                        self.write_response(501, "5.5.0 AUTH cancelled by client", Some(line))
                            .await?;
                        continue;
                    }

                    match BASE64.decode(response.as_bytes()) {
                        Ok(payload) => {
                            // RFC 4616 says that the message is:
                            // [authzid] NUL authcid NUL passwd
                            let fields: Vec<_> = payload.split(|&b| b == 0).collect();
                            let (authz, authc, pass) = match fields.len() {
                                3 => (
                                    std::str::from_utf8(&fields[0]),
                                    std::str::from_utf8(&fields[1]),
                                    std::str::from_utf8(&fields[2]),
                                ),
                                _ => {
                                    self.write_response(
                                        501,
                                        "5.5.2 Invalid decoded PLAIN response",
                                        Some(response),
                                    )
                                    .await?;
                                    continue;
                                }
                            };

                            let (authz, authc, pass) = match (authz, authc, pass) {
                                (Ok(a), Ok(b), Ok(c)) => (a, b, c),
                                _ => {
                                    self.write_response(
                                        501,
                                        "5.5.2 Invalid UTF8 in decoded PLAIN response",
                                        Some(response),
                                    )
                                    .await?;
                                    continue;
                                }
                            };

                            // If no authorization id was set, assume the same as
                            // the authenticated id
                            let authz = if authz.is_empty() { authc } else { authz };

                            match self
                                .call_callback(
                                    "smtp_server_auth_plain",
                                    (authz, authc, pass, self.meta.clone()),
                                )
                                .await?
                            {
                                Err(rej) => {
                                    self.write_response(rej.code, rej.message, Some(response))
                                        .await?;
                                    continue;
                                }
                                Ok(false) => {
                                    self.write_response(535, "5.7.8 AUTH invalid", Some(response))
                                        .await?;
                                }
                                Ok(true) => {
                                    self.authorization_id.replace(authz.to_string());
                                    self.authentication_id.replace(authc.to_string());
                                    self.meta.set_meta("authz_id", authz);
                                    self.meta.set_meta("authn_id", authc);

                                    self.write_response(235, "2.7.0 AUTH OK!", None).await?;
                                }
                            }
                        }
                        Err(_) => {
                            self.write_response(
                                501,
                                "5.5.2 Invalid base64 response",
                                Some(response),
                            )
                            .await?;
                            continue;
                        }
                    }
                }
                Ok(Command::Ehlo(domain)) => {
                    let domain = domain.to_string();

                    let mut extensions = vec!["PIPELINING", "ENHANCEDSTATUSCODES"];
                    if !self.tls_active {
                        extensions.push("STARTTLS");
                    } else {
                        extensions.push("AUTH PLAIN");
                    }

                    let extensions = match self
                        .call_callback::<Option<Vec<String>>, _, _>(
                            "smtp_server_ehlo",
                            (domain.clone(), self.meta.clone(), extensions.clone()),
                        )
                        .await?
                    {
                        Err(rej) => {
                            self.write_response(rej.code, rej.message, Some(line))
                                .await?;
                            continue;
                        }
                        Ok(None) => extensions.join("\n"),
                        Ok(Some(ext)) => ext.join("\n"),
                    };

                    self.write_response(
                        250,
                        format!("{} Aloha {domain}\n{extensions}", self.params.hostname,),
                        None,
                    )
                    .await?;

                    self.meta.set_meta("ehlo_domain", domain.clone());
                    self.said_hello.replace(domain);
                }
                Ok(Command::Helo(domain)) => {
                    let domain = domain.to_string();

                    if let Err(rej) = self
                        .call_callback::<(), _, _>(
                            "smtp_server_ehlo",
                            (domain.clone(), self.meta.clone()),
                        )
                        .await?
                    {
                        self.write_response(rej.code, rej.message, Some(line))
                            .await?;
                        continue;
                    }
                    self.write_response(250, format!("Hello {domain}!"), None)
                        .await?;
                    self.meta.set_meta("ehlo_domain", domain.clone());
                    self.said_hello.replace(domain);
                }
                Ok(Command::MailFrom {
                    address,
                    parameters: _,
                }) => {
                    if self.state.is_some() {
                        self.write_response(
                            503,
                            "5.5.0 MAIL FROM already issued; you must RSET first",
                            Some(line),
                        )
                        .await?;
                        continue;
                    }
                    let address = EnvelopeAddress::parse(&address.to_string())?;
                    if let Err(rej) = self
                        .call_callback::<(), _, _>(
                            "smtp_server_mail_from",
                            (address.clone(), self.meta.clone()),
                        )
                        .await?
                    {
                        self.write_response(rej.code, rej.message, Some(line))
                            .await?;
                        continue;
                    }

                    self.state.replace(TransactionState {
                        sender: address.clone(),
                        recipients: vec![],
                        _timer: TXN_LATENCY.start_timer(),
                    });
                    self.write_response(250, format!("OK {address:?}"), None)
                        .await?;
                }
                Ok(Command::RcptTo {
                    address,
                    parameters: _,
                }) => {
                    if self.state.is_none() {
                        self.write_response(
                            503,
                            "5.5.0 MAIL FROM must be issued first",
                            Some(line),
                        )
                        .await?;
                        continue;
                    }
                    let address = EnvelopeAddress::parse(&address.to_string())?;

                    let sender = self.state.as_ref().unwrap().sender.clone();
                    let relay_disposition = self.check_relaying(&sender, &address).await?;

                    if !relay_disposition.accept_rcpt_to() {
                        self.write_response(
                            550,
                            format!("5.7.1 relaying not permitted for {}", self.peer_address),
                            Some(line),
                        )
                        .await?;
                        continue;
                    }

                    if let Some(state) = &self.state {
                        if state.recipients.len() == self.params.max_recipients_per_message {
                            self.write_response(451, "4.5.3 too many recipients", Some(line))
                                .await?;
                            continue;
                        }

                        if self.rcpt_count == self.params.max_messages_per_connection {
                            if state.recipients.is_empty() {
                                self.write_response(
                                    421,
                                    format!(
                                        "4.5.3 {} too many recipients on this connection",
                                        self.params.hostname
                                    ),
                                    Some(line),
                                )
                                .await?;
                                return Ok(());
                            } else {
                                self.write_response(
                                    451,
                                    "4.5.3 too many recipients on this connection",
                                    Some(line),
                                )
                                .await?;
                                continue;
                            }
                        }
                    } else {
                        self.write_response(
                            503,
                            "5.5.0 MAIL FROM must be issued first",
                            Some(line),
                        )
                        .await?;
                        continue;
                    }
                    self.rcpt_count += 1;
                    if let Err(rej) = self
                        .call_callback::<(), _, _>(
                            "smtp_server_rcpt_to",
                            (address.clone(), self.meta.clone()),
                        )
                        .await?
                    {
                        self.write_response(rej.code, rej.message, Some(line))
                            .await?;
                        continue;
                    }
                    self.write_response(250, format!("OK {address:?}"), None)
                        .await?;
                    self.state
                        .as_mut()
                        .expect("checked state above")
                        .recipients
                        .push(address);
                }
                Ok(Command::Data) => {
                    if self.state.is_none() {
                        self.write_response(
                            503,
                            "5.5.0 MAIL FROM must be issued first",
                            Some(line),
                        )
                        .await?;
                        continue;
                    }
                    if self
                        .state
                        .as_ref()
                        .map(|s| s.recipients.is_empty())
                        .unwrap_or(true)
                    {
                        self.write_response(503, "5.5.0 RCPT TO must be issued first", Some(line))
                            .await?;
                        continue;
                    }

                    self.write_response(354, "Send body; end with CRLF.CRLF", None)
                        .await?;

                    let read_data_timer = READ_DATA_LATENCY.start_timer();
                    let data = match self.read_data().await? {
                        ReadData::Disconnected => return Ok(()),
                        ReadData::Data(data) => data,
                        ReadData::TooBig => {
                            self.write_response(552, "5.3.4 message too big", Some(line))
                                .await?;
                            continue;
                        }
                        ReadData::TooLong => {
                            self.write_response(500, "5.2.3 line too long", Some(line))
                                .await?;
                            continue;
                        }
                        ReadData::TimedOut => {
                            self.write_response(
                                421,
                                format!("4.3.2 {} idle too long", self.params.hostname),
                                Some(line),
                            )
                            .await?;
                            return Ok(());
                        }
                        ReadData::ShuttingDown => {
                            self.write_response(
                                421,
                                format!("4.3.2 {} shutting down", self.params.hostname),
                                Some(line),
                            )
                            .await?;
                            return Ok(());
                        }
                    };
                    read_data_timer.stop_and_record();

                    let _process_data_timer = PROCESS_DATA_LATENCY.start_timer();
                    self.process_data(data).await?;
                }
                Ok(Command::Rset) => {
                    self.state.take();
                    self.write_response(250, "Reset state", None).await?;
                }
                Ok(Command::Noop(_)) => {
                    self.write_response(250, "the goggles do nothing", None)
                        .await?;
                }
                Ok(Command::Vrfy(_) | Command::Expn(_) | Command::Help(_) | Command::Lhlo(_)) => {
                    self.write_response(502, format!("5.5.1 Command unimplemented"), Some(line))
                        .await?;
                }
                Ok(Command::DataDot) => unreachable!(),
            }
        }
    }

    async fn process_data(&mut self, mut data: Vec<u8>) -> anyhow::Result<()> {
        self.reception_count.inc();
        self.global_reception_count.inc();
        let state = self
            .state
            .take()
            .ok_or_else(|| anyhow!("transaction state is impossibly not set!?"))?;

        tracing::trace!(?state);

        let lone_lf = mailparsing::has_lone_cr_or_lf(&data);
        if lone_lf {
            match self.params.invalid_line_endings {
                ConformanceDisposition::Deny => {
                    self.write_response(
                        552,
                        "5.6.0 message data must use CRLF for line endings",
                        Some("DATA".into()),
                    )
                    .await?;
                    return Ok(());
                }
                ConformanceDisposition::Allow => {
                    SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                        conn_meta: self.meta.clone_inner(),
                        payload: SmtpServerTraceEventPayload::Diagnostic {
                            level: Level::INFO,
                            message: "Allowing invalid line endings in DATA".to_string(),
                        },
                        when: Utc::now(),
                    });
                }
                ConformanceDisposition::Fix => {
                    SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                        conn_meta: self.meta.clone_inner(),
                        payload: SmtpServerTraceEventPayload::Diagnostic {
                            level: Level::INFO,
                            message: "Fixed line endings in DATA".to_string(),
                        },
                        when: Utc::now(),
                    });

                    mailparsing::normalize_crlf_in_place(&mut data);
                }
            }
        }

        let mut ids = vec![];

        // If anything decides to reject at this phase, it needs to apply to
        // the entire batch, so we make a first pass to accumulate the messages
        // here. If anything rejects, we return before we've committed to doing
        // any real work
        let mut accepted_messages = vec![];

        let datestamp = Utc::now().to_rfc2822();

        for recip in state.recipients {
            let id = SpoolId::new();
            let protocol = "ESMTP"; // FIXME: update SmtpServer ctor if we change this.
                                    // OR: just read this from self.meta?

            let mut body = if self.params.trace_headers.received_header {
                let received = {
                    let from_domain = self.said_hello.as_deref().unwrap_or("unspecified");
                    let peer_address = self.peer_address.ip();
                    let my_address = self.my_address.ip();
                    let hostname = &self.params.hostname;
                    let recip = recip.to_string();
                    format!(
                        "Received: from {from_domain} ({peer_address})\r\n  \
                                       by {hostname} (KumoMTA {my_address}) \r\n  \
                                       with {protocol} id {id} for <{recip}>;\r\n  \
                                       {datestamp}\r\n"
                    )
                };

                let mut body = Vec::with_capacity(data.len() + received.len());
                body.extend_from_slice(received.as_bytes());
                body
            } else {
                Vec::with_capacity(data.len())
            };

            body.extend_from_slice(&data);

            let message = Message::new_dirty(
                id,
                state.sender.clone(),
                recip,
                self.meta.clone_inner(),
                Arc::new(body.into_boxed_slice()),
            )?;

            if self.params.deferred_queue {
                message.set_meta("queue", DEFERRED_QUEUE_NAME)?;
            } else {
                if let Err(rej) = self
                    .call_callback_sig(&SMTP_SERVER_MSG_RX, (message.clone(), self.meta.clone()))
                    .await?
                {
                    // Rejecting any one message from a batch in
                    // smtp_server_message_received will reject the
                    // entire batch
                    self.write_response(rej.code, rej.message, Some("DATA".into()))
                        .await?;
                    return Ok(());
                }
            }
            accepted_messages.push(message);
        }

        // At this point we've nominally accepted the batch; let's
        // get to work on logging and injecting into the queues

        let mut messages = vec![];
        let mut was_arf_or_oob = false;
        let mut black_holed = false;

        for message in accepted_messages {
            self.params.trace_headers.apply_supplemental(&message)?;

            ids.push(message.id().to_string());

            let queue_name = message.get_queue_name()?;

            let relay_disposition = self
                .check_relaying(&message.sender()?, &message.recipient()?)
                .await?;

            SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                conn_meta: self.meta.clone_inner(),
                payload: SmtpServerTraceEventPayload::MessageDisposition {
                    relay: relay_disposition.relay,
                    log_arf: relay_disposition.log_arf,
                    log_oob: relay_disposition.log_oob,
                    queue: queue_name.clone(),
                    meta: message.get_meta_obj().unwrap_or(serde_json::Value::Null),
                    sender: message
                        .sender()
                        .map(|s| s.to_string())
                        .expect("have sender"),
                    recipient: message
                        .recipient()
                        .map(|s| s.to_string())
                        .expect("have recipient"),
                    id: *message.id(),
                },
                when: Utc::now(),
            });

            if queue_name != "null" {
                if relay_disposition.relay && !self.params.deferred_spool {
                    message.save().await?;
                }
            }

            if relay_disposition.log_arf && matches!(message.parse_rfc5965(), Ok(Some(_))) {
                was_arf_or_oob = true;
            } else if relay_disposition.log_oob && matches!(message.parse_rfc3464(), Ok(Some(_))) {
                was_arf_or_oob = true;
            }

            log_disposition(LogDisposition {
                kind: RecordType::Reception,
                msg: message.clone(),
                site: "",
                peer_address: Some(&ResolvedAddress {
                    name: self.said_hello.as_deref().unwrap_or("").to_string(),
                    addr: self.peer_address.ip().into(),
                }),
                response: Response {
                    code: 250,
                    enhanced_code: None,
                    command: None,
                    content: "".to_string(),
                },
                egress_pool: None,
                egress_source: None,
                relay_disposition: Some(relay_disposition),
                delivery_protocol: None,
                tls_info: None, // TODO: populate with peer info
                source_address: None,
                provider: None,
                session_id: Some(self.session_id),
            })
            .await;
            if queue_name != "null" {
                if relay_disposition.relay {
                    messages.push((queue_name, message));
                }
            } else {
                black_holed = true;
            }
        }

        let relayed_any = !messages.is_empty();
        let mut failed = vec![];

        for (queue_name, msg) in messages {
            let id = *msg.id();
            if let Err(err) =
                QueueManager::insert_or_unwind(&queue_name, msg.clone(), self.params.deferred_spool)
                    .await
            {
                // Record the error message for later reporting
                failed.push(format!("{id}: {err:#}"));

                // And a diagnostic for the tracer, if any.
                SmtpServerTraceManager::submit(|| SmtpServerTraceEvent {
                    conn_meta: self.meta.clone_inner(),
                    payload: SmtpServerTraceEventPayload::Diagnostic {
                        level: Level::ERROR,
                        message: format!("QueueManager::insert failed for {}: {err:#}", msg.id()),
                    },
                    when: Utc::now(),
                });
            }
        }

        if !black_holed && !relayed_any && !was_arf_or_oob {
            self.write_response(550, "5.7.1 relaying not permitted", Some("DATA".into()))
                .await?;
        } else if !failed.is_empty() && failed.len() == ids.len() {
            // All potentials failed, report error.
            // This will map to a 421 and get traced and logged appropriately
            anyhow::bail!(
                "QueueManager::insert failed for {} messages: {}",
                failed.len(),
                failed.join(", ")
            );
        } else {
            let disposition = if !failed.is_empty() { "PARTIAL" } else { "OK" };

            let ids = ids.join(" ");
            self.write_response(250, format!("{disposition} ids={ids}"), None)
                .await?;
        }

        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct ConnectionMetaData {
    map: Arc<Mutex<serde_json::Value>>,
}

impl ConnectionMetaData {
    pub fn new() -> Self {
        Self {
            map: Arc::new(Mutex::new(json!({}))),
        }
    }

    pub fn set_meta<N: Into<String>, V: Into<serde_json::Value>>(&mut self, name: N, value: V) {
        let mut map = self.map.lock();
        let meta = map.as_object_mut().expect("map is always an object");
        meta.insert(name.into(), value.into());
    }

    pub fn get_meta<N: AsRef<str>>(&self, name: N) -> Option<serde_json::Value> {
        let map = self.map.lock();
        let meta = map.as_object().expect("map is always an object");
        meta.get(name.as_ref()).cloned()
    }

    pub fn clone_inner(&self) -> serde_json::Value {
        self.map.lock().clone()
    }
}

impl UserData for ConnectionMetaData {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut(
            "set_meta",
            move |_, this, (name, value): (String, mlua::Value)| {
                let value = serde_json::value::to_value(value).map_err(any_err)?;
                this.set_meta(name, value);
                Ok(())
            },
        );

        methods.add_method("get_meta", move |lua, this, name: String| {
            match this.get_meta(name) {
                Some(value) => Ok(lua.to_value_with(&value, serialize_options())?),
                None => Ok(mlua::Value::Nil),
            }
        });
    }
}

#[derive(Error, Debug)]
#[error("Error writing to client")]
struct WriteError;

/// The maximum line length defined by the SMTP RFCs
const MAX_LINE_LEN: usize = 998;

#[derive(PartialEq)]
enum ReadLine {
    Line(String),
    TooLong,
    ShuttingDown,
    TimedOut,
    Disconnected,
}

#[derive(PartialEq)]
enum ReadData {
    Data(Vec<u8>),
    TooLong,
    TooBig,
    ShuttingDown,
    TimedOut,
    Disconnected,
}

fn unstuff(data: Vec<u8>) -> Vec<u8> {
    static CRLFDOTDOT: LazyLock<Finder> = LazyLock::new(|| Finder::new("\r\n.."));
    let mut stuffing_finder = CRLFDOTDOT.find_iter(&data);
    if let Some(stuffed) = stuffing_finder.next() {
        let mut unstuffed = Vec::with_capacity(data.len());
        unstuffed.extend_from_slice(&data[0..stuffed + 3]);
        let mut last_pos = stuffed + 4;
        while let Some(stuffed) = stuffing_finder.next() {
            unstuffed.extend_from_slice(&data[last_pos..stuffed + 3]);
            last_pos = stuffed + 4;
        }
        unstuffed.extend_from_slice(&data[last_pos..]);
        return unstuffed;
    }
    data
}

fn check_line_lengths(data: &[u8], limit: usize) -> bool {
    let mut last_index = 0;
    for idx in CRLF.find_iter(data) {
        if idx - last_index > limit {
            return false;
        }
        last_index = idx;
    }
    data.len() - last_index <= limit
}

pub fn make_deferred_queue_config() -> anyhow::Result<QueueConfig> {
    Ok(QueueConfig {
        protocol: DeliveryProto::DeferredSmtpInjection,
        retry_interval: Duration::from_secs(60),
        ..QueueConfig::default()
    })
}

#[derive(Debug)]
pub struct DeferredSmtpInjectionDispatcher {
    connection: Option<MetricsWrappedConnection<()>>,
}

impl DeferredSmtpInjectionDispatcher {
    pub fn new() -> Self {
        Self { connection: None }
    }
}

#[async_trait]
impl QueueDispatcher for DeferredSmtpInjectionDispatcher {
    async fn close_connection(&mut self, _dispatcher: &mut Dispatcher) -> anyhow::Result<bool> {
        match self.connection.take() {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }

    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        if self.connection.is_none() {
            self.connection
                .replace(dispatcher.metrics.wrap_connection(()));
        }
        Ok(())
    }

    async fn have_more_connection_candidates(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        false
    }

    async fn deliver_message(
        &mut self,
        mut msgs: Vec<Message>,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()> {
        // parse out the inject payload and run it
        anyhow::ensure!(
            msgs.len() == 1,
            "DeferredSmtpInjectionDispatcher only supports a batch size of 1"
        );
        let msg = msgs.pop().expect("just verified that there is one");

        msg.set_meta("queue", serde_json::Value::Null)?;
        let meta = ConnectionMetaData {
            map: Arc::new(msg.get_meta_obj()?.into()),
        };

        let mut config = load_config().await?;

        let mut response = match config
            .async_call_callback(&DEFERRED_SMTP_SERVER_MSG_INJECT, (msg.clone(), meta))
            .await
        {
            Ok(_) => Response {
                code: 250,
                enhanced_code: None,
                content: "ok".to_string(),
                command: None,
            },
            Err(err) => {
                if let Some(rej) = RejectError::from_anyhow(&err) {
                    Response {
                        code: rej.code,
                        enhanced_code: None,
                        content: rej.message,
                        command: None,
                    }
                } else {
                    Response {
                        code: 450,
                        enhanced_code: None,
                        content: format!("{err:#}"),
                        command: None,
                    }
                }
            }
        };

        if response.code == 250 {
            msg.set_due(None).await?;
            let queue_name = msg.get_queue_name()?;
            if let Err(err) = QueueManager::insert(&queue_name, msg.clone()).await {
                response = Response {
                    code: 450,
                    enhanced_code: None,
                    content: format!("{err:#}"),
                    command: None,
                };
            }
        }

        let code = response.code;
        let kind = if code == 250 {
            RecordType::DeferredInjectionRebind
        } else if code >= 500 {
            RecordType::Bounce
        } else {
            RecordType::TransientFailure
        };

        log_disposition(LogDisposition {
            kind,
            msg: msg.clone(),
            site: &dispatcher.name,
            peer_address: None,
            response: response.clone(),
            egress_pool: None,
            egress_source: None,
            relay_disposition: None,
            delivery_protocol: Some("DeferredSmtpInjection"),
            tls_info: None,
            source_address: None,
            provider: None,
            session_id: None,
        })
        .await;

        if code == 250 {
            // Message has been re-queued
            let _ = dispatcher.msgs.pop();
            dispatcher.metrics.inc_delivered();
        } else if code >= 500 {
            // Policy decided to permanently fail it
            SpoolManager::remove_from_spool(*msg.id()).await?;
            let _ = dispatcher.msgs.pop();
            dispatcher.metrics.inc_fail();
        } else {
            dispatcher.metrics.inc_transfail();

            // Ensure that we get another crack at it later
            msg.set_meta("queue", DEFERRED_QUEUE_NAME)?;
            let _ = dispatcher.msgs.pop();
            spawn(
                "requeue message".to_string(),
                QueueManager::requeue_message(msg, IncrementAttempts::Yes, None, response),
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn unstuffer() {
        let stuffed = b"hello\r\n..dot\r\nthere\r\n..more dot".to_vec();
        assert_eq!(
            unstuff(stuffed).as_slice(),
            b"hello\r\n.dot\r\nthere\r\n.more dot"
        );

        let stuffed = b"hello".to_vec();
        assert_eq!(unstuff(stuffed).as_slice(), b"hello");
    }

    #[test]
    fn line_lengths() {
        assert!(check_line_lengths(b"hello", 78));
        assert!(check_line_lengths(b"hello", 5));
        assert!(!check_line_lengths(b"hello", 4));

        assert!(check_line_lengths(
            b"hello there\r\nanother line over there\r\n",
            78
        ));
        assert!(!check_line_lengths(
            b"hello there\r\nanother line over there\r\n",
            12
        ));
    }
}
