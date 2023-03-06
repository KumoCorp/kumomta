use crate::cidrset::{CidrSet, IpCidr};
use crate::lifecycle::{Activity, ShutdownSubcription};
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::mx::ResolvedAddress;
use crate::queue::QueueManager;
use crate::runtime::{rt_spawn, spawn_local};
use crate::spool::SpoolManager;
use anyhow::{anyhow, Context};
use chrono::Utc;
use config::{load_config, LuaConfig};
use data_loader::KeySource;
use domain_map::DomainMap;
use message::{EnvelopeAddress, Message};
use mlua::ToLuaMulti;
use once_cell::sync::OnceCell;
use prometheus::IntGauge;
use rfc5321::{AsyncReadAndWrite, BoxedAsyncReadAndWrite, Command, Response};
use rustls::ServerConfig;
use serde::Deserialize;
use serde_json::json;
use spool::SpoolId;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, instrument};

#[derive(Deserialize, Clone, Debug, Default)]
pub struct EsmtpDomain {
    #[serde(default)]
    pub log_oob: bool,
    #[serde(default)]
    pub log_arf: bool,
    #[serde(default)]
    pub relay_to: bool,
    #[serde(default)]
    pub relay_from: CidrSet,
}

#[derive(Deserialize, Clone, Debug)]
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
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
pub struct EsmtpListenerParams {
    #[serde(default = "EsmtpListenerParams::default_listen")]
    pub listen: String,
    #[serde(default = "EsmtpListenerParams::default_hostname")]
    pub hostname: String,
    #[serde(default = "EsmtpListenerParams::default_relay_hosts")]
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
    pub domains: DomainMap<EsmtpDomain>,

    #[serde(default)]
    pub trace_headers: TraceHeaders,

    #[serde(
        default = "EsmtpListenerParams::default_client_timeout",
        with = "humantime_serde"
    )]
    pub client_timeout: Duration,

    #[serde(skip)]
    tls_config: OnceCell<Arc<ServerConfig>>,

    #[serde(skip)]
    connection_gauge: OnceCell<IntGauge>,

    #[serde(default = "EsmtpListenerParams::default_max_messages_per_connection")]
    max_messages_per_connection: usize,
    #[serde(default = "EsmtpListenerParams::default_max_recipients_per_message")]
    max_recipients_per_message: usize,
}

impl EsmtpListenerParams {
    fn default_max_messages_per_connection() -> usize {
        10_000
    }

    fn default_max_recipients_per_message() -> usize {
        1024
    }

    fn default_client_timeout() -> Duration {
        Duration::from_secs(60)
    }

    fn default_relay_hosts() -> CidrSet {
        CidrSet::new(vec![
            IpCidr::new("127.0.0.1".parse().unwrap(), 32).unwrap(),
            IpCidr::new("::1".parse().unwrap(), 128).unwrap(),
        ])
    }

    fn default_listen() -> String {
        "127.0.0.1:2025".to_string()
    }

    fn default_hostname() -> String {
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

        let config = crate::tls_helpers::make_server_config(
            &self.hostname,
            &self.tls_private_key,
            &self.tls_certificate,
        )
        .await?;

        // If we race to create, take the winner's version
        match self.tls_config.try_insert(config) {
            Ok(config) | Err((config, _)) => Ok(TlsAcceptor::from(config.clone())),
        }
    }

    pub fn connection_gauge(&self) -> &IntGauge {
        self.connection_gauge
            .get_or_init(|| crate::metrics_helper::connection_gauge_for_service("esmtp_listener"))
    }

    #[instrument]
    pub async fn run(self) -> anyhow::Result<()> {
        // Pre-create the acceptor so that we can share it across
        // the various listeners
        self.build_tls_acceptor().await?;
        self.connection_gauge();

        let listener = TcpListener::bind(&self.listen)
            .await
            .with_context(|| format!("failed to bind to {}", self.listen))?;

        tracing::debug!("smtp listener on {}", self.listen);
        let mut shutting_down = ShutdownSubcription::get();

        loop {
            tokio::select! {
                _ = shutting_down.shutting_down() => {
                    println!("smtp listener on {} -> stopping", self.listen);
                    return Ok(());
                }
                result = listener.accept() => {
                    let (socket, peer_address) = result?;
                    let my_address = socket.local_addr()?;
                    let params = self.clone();
                    rt_spawn(
                        format!("SmtpServer {peer_address:?}"),
                        move || Ok(async move {
                            if let Err(err) =
                                SmtpServer::run(socket, my_address, peer_address, params).await
                                {
                                    tracing::error!("SmtpServer::run: {err:#}");
                            }
                    }))?;
                }
            };
        }
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

pub struct SmtpServer {
    socket: Option<BoxedAsyncReadAndWrite>,
    state: Option<TransactionState>,
    said_hello: Option<String>,
    config: LuaConfig,
    peer_address: SocketAddr,
    my_address: SocketAddr,
    tls_active: bool,
    read_buffer: Vec<u8>,
    params: EsmtpListenerParams,
    shutdown: ShutdownSubcription,
    rcpt_count: usize,
}

#[derive(Debug)]
struct TransactionState {
    sender: EnvelopeAddress,
    recipients: Vec<EnvelopeAddress>,
    meta: serde_json::Value,
}

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
    #[instrument]
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
        let config = load_config().await?;
        let mut server = SmtpServer {
            socket: Some(socket),
            state: None,
            said_hello: None,
            config,
            peer_address,
            my_address,
            tls_active: false,
            read_buffer: Vec::with_capacity(1024),
            params,
            shutdown: ShutdownSubcription::get(),
            rcpt_count: 0,
        };

        server.params.connection_gauge().inc();
        if let Err(err) = server.process().await {
            if err.downcast_ref::<WriteError>().is_none() {
                error!("Error in SmtpServer: {err:#}");
                server
                    .write_response(
                        421,
                        format!("4.3.0 {} technical difficulties", server.params.hostname),
                    )
                    .await
                    .ok();
            }
        }
        server.params.connection_gauge().dec();
        Ok(())
    }

    fn peer_in_cidr_list(&self, cidr: &CidrSet) -> bool {
        cidr.contains(self.peer_address.ip())
    }

    fn check_relaying(
        &self,
        sender: &EnvelopeAddress,
        recipient: &EnvelopeAddress,
    ) -> RelayDisposition {
        let relay_hosts_allowed = self.peer_in_cidr_list(&self.params.relay_hosts);

        let sender_domain = sender.domain();
        let mut relay_from_allowed = false;
        if let Some(dom) = self.params.domains.get(sender_domain) {
            relay_from_allowed = self.peer_in_cidr_list(&dom.relay_from);
        }

        let recipient_domain = recipient.domain();
        let mut relay_to_allowed = None;
        let mut log_arf = false;
        let mut log_oob = false;
        if let Some(dom) = self.params.domains.get(recipient_domain) {
            relay_to_allowed.replace(dom.relay_to);
            log_arf = dom.log_arf;
            log_oob = dom.log_oob;
        }

        let relay = if relay_to_allowed == Some(false) {
            // Explicitly denied from relaying
            false
        } else if relay_hosts_allowed || relay_from_allowed || relay_to_allowed == Some(true) {
            true
        } else {
            false
        };

        RelayDisposition {
            relay,
            log_arf,
            log_oob,
        }
    }

    async fn write_response<S: AsRef<str>>(
        &mut self,
        status: u16,
        message: S,
    ) -> Result<(), WriteError> {
        if let Some(socket) = self.socket.as_mut() {
            let mut lines = message.as_ref().lines().peekable();
            while let Some(line) = lines.next() {
                let is_last = lines.peek().is_none();
                let sep = if is_last { ' ' } else { '-' };
                let text = format!("{status}{sep}{line}\r\n");
                socket
                    .write(text.as_bytes())
                    .await
                    .map_err(|_| WriteError {})?;
            }
            socket.flush().await.map_err(|_| WriteError {})?;
        }
        Ok(())
    }

    fn check_shutdown(&self) -> bool {
        if self.read_buffer.is_empty() {
            Activity::get_opt().is_none()
        } else {
            false
        }
    }

    async fn read_line(&mut self) -> anyhow::Result<ReadLine> {
        let mut too_long = false;
        loop {
            let mut iter = self.read_buffer.iter().enumerate();
            while let Some((i, &b)) = iter.next() {
                if b != b'\r' {
                    continue;
                }
                if let Some((_, b'\n')) = iter.next() {
                    if too_long {
                        self.read_buffer.drain(0..i + 2);
                        return Ok(ReadLine::TooLong);
                    }

                    let line = String::from_utf8(self.read_buffer[0..i].to_vec());
                    self.read_buffer.drain(0..i + 2);
                    return Ok(ReadLine::Line(line?));
                }
            }
            if self.read_buffer.len() > MAX_LINE_LEN {
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
                            return Ok(ReadLine::Disconnected);
                        }
                        Ok(size) if size == 0 => {
                            return Ok(ReadLine::Disconnected);
                        }
                        Ok(size) => {
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

    pub async fn call_callback<'lua, S: AsRef<str>, A: ToLuaMulti<'lua> + Clone>(
        &'lua mut self,
        name: S,
        args: A,
    ) -> anyhow::Result<Result<(), RejectError>> {
        match self.config.async_call_callback(name, args).await {
            Ok(()) => Ok(Ok(())),
            Err(err) => {
                if let Some(rej) = RejectError::from_anyhow(&err) {
                    Ok(Err(rej))
                } else {
                    Err(err)
                }
            }
        }
    }

    #[instrument(skip(self))]
    async fn process(&mut self) -> anyhow::Result<()> {
        let _activity = match Activity::get_opt() {
            None => {
                // Can't accept any messages while we're shutting down
                self.write_response(421, format!("4.3.2 {} shutting down", self.params.hostname))
                    .await?;
                return Ok(());
            }
            Some(a) => a,
        };
        if !SpoolManager::get().await.spool_started() {
            // Can't accept any messages until the spool is finished enumerating,
            // else we risk re-injecting messages received during enumeration.
            self.write_response(
                421,
                format!("{} 4.3.2 Hold on just a moment!", self.params.hostname),
            )
            .await?;
            return Ok(());
        }

        self.write_response(
            220,
            format!("{} {}", self.params.hostname, self.params.banner),
        )
        .await?;
        loop {
            if self.check_shutdown() {
                self.write_response(421, format!("4.3.2 {} shutting down", self.params.hostname))
                    .await?;
                return Ok(());
            }

            let line = match self.read_line().await? {
                ReadLine::Disconnected => return Ok(()),
                ReadLine::Line(line) => line,
                ReadLine::TimedOut => {
                    self.write_response(
                        421,
                        format!("4.3.2 {} idle too long", self.params.hostname),
                    )
                    .await?;
                    return Ok(());
                }
                ReadLine::ShuttingDown => {
                    self.write_response(
                        421,
                        format!("4.3.2 {} shutting down", self.params.hostname),
                    )
                    .await?;
                    return Ok(());
                }
                ReadLine::TooLong => {
                    self.write_response(500, "5.2.3 line too long").await?;
                    continue;
                }
            };

            match Command::parse(&line) {
                Err(err) => {
                    self.write_response(
                        501,
                        format!("Syntax error in command or arguments: {err}"),
                    )
                    .await?;
                }
                Ok(Command::Quit) => {
                    self.write_response(221, "So long, and thanks for all the fish!")
                        .await?;
                    return Ok(());
                }
                Ok(Command::StartTls) => {
                    if self.tls_active {
                        self.write_response(501, "Cannot STARTTLS as TLS is already active")
                            .await?;
                        continue;
                    }
                    self.write_response(220, "Ready to Start TLS").await?;
                    let acceptor = self.params.build_tls_acceptor().await?;
                    let socket = acceptor.accept(self.socket.take().unwrap()).await?;
                    let socket: BoxedAsyncReadAndWrite = Box::new(socket);
                    self.socket.replace(socket);
                    self.tls_active = true;
                }
                Ok(Command::Ehlo(domain)) => {
                    let domain = domain.to_string();

                    if let Err(rej) = self
                        .call_callback("smtp_server_ehlo", domain.clone())
                        .await?
                    {
                        self.write_response(rej.code, rej.message).await?;
                        continue;
                    }

                    let mut extensions = vec!["PIPELINING", "ENHANCEDSTATUSCODES"];
                    if !self.tls_active {
                        extensions.push("STARTTLS");
                    }

                    self.write_response(
                        250,
                        format!(
                            "{} Aloha {domain}\n{}",
                            self.params.hostname,
                            extensions.join("\n"),
                        ),
                    )
                    .await?;
                    self.said_hello.replace(domain);
                }
                Ok(Command::Helo(domain)) => {
                    let domain = domain.to_string();

                    if let Err(rej) = self
                        .call_callback("smtp_server_ehlo", domain.clone())
                        .await?
                    {
                        self.write_response(rej.code, rej.message).await?;
                        continue;
                    }
                    self.write_response(250, format!("Hello {domain}!")).await?;
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
                        )
                        .await?;
                        continue;
                    }

                    let address = EnvelopeAddress::parse(&address.to_string())?;
                    if let Err(rej) = self
                        .call_callback("smtp_server_mail_from", address.clone())
                        .await?
                    {
                        self.write_response(rej.code, rej.message).await?;
                        continue;
                    }

                    self.state.replace(TransactionState {
                        sender: address.clone(),
                        recipients: vec![],
                        meta: serde_json::json!({}),
                    });
                    self.write_response(250, format!("OK {address:?}")).await?;
                }
                Ok(Command::RcptTo {
                    address,
                    parameters: _,
                }) => {
                    let address = EnvelopeAddress::parse(&address.to_string())?;
                    let relay_disposition =
                        self.check_relaying(&self.state.as_ref().unwrap().sender, &address);

                    if !relay_disposition.accept_rcpt_to() {
                        self.write_response(
                            550,
                            format!("5.7.1 relaying not permitted for {}", self.peer_address),
                        )
                        .await?;
                        continue;
                    }

                    if let Some(state) = &self.state {
                        if state.recipients.len() == self.params.max_recipients_per_message {
                            self.write_response(451, "4.5.3 too many recipients")
                                .await?;
                            continue;
                        }

                        if self.rcpt_count == self.params.max_messages_per_connection {
                            if state.recipients.is_empty() {
                                self.write_response(
                                    451,
                                    format!(
                                        "4.5.3 {} too many recipients on this connection",
                                        self.params.hostname
                                    ),
                                )
                                .await?;
                                return Ok(());
                            } else {
                                self.write_response(451, "4.5.3 too many on this conn")
                                    .await?;
                                continue;
                            }
                        }
                    } else {
                        self.write_response(503, "5.5.0 MAIL FROM must be issued first")
                            .await?;
                        continue;
                    }
                    self.rcpt_count += 1;
                    if let Err(rej) = self
                        .call_callback("smtp_server_rcpt_to", address.clone())
                        .await?
                    {
                        self.write_response(rej.code, rej.message).await?;
                        continue;
                    }
                    self.write_response(250, format!("OK {address:?}")).await?;
                    self.state
                        .as_mut()
                        .expect("checked state above")
                        .recipients
                        .push(address);
                }
                Ok(Command::Data) => {
                    if self.state.is_none() {
                        self.write_response(503, "5.5.0 MAIL FROM must be issued first")
                            .await?;
                        continue;
                    }
                    if self
                        .state
                        .as_ref()
                        .map(|s| s.recipients.is_empty())
                        .unwrap_or(true)
                    {
                        self.write_response(503, "5.5.0 RCPT TO must be issued first")
                            .await?;
                        continue;
                    }

                    let mut data = vec![];
                    let mut too_long = false;

                    self.write_response(354, "Send body; end with CRLF.CRLF")
                        .await?;

                    loop {
                        let line = match self.read_line().await? {
                            ReadLine::Disconnected => return Ok(()),
                            ReadLine::Line(line) => line,
                            ReadLine::TooLong => {
                                too_long = true;
                                data.clear();
                                continue;
                            }
                            ReadLine::TimedOut => {
                                self.write_response(
                                    421,
                                    format!("4.3.2 {} idle too long", self.params.hostname),
                                )
                                .await?;
                                return Ok(());
                            }
                            ReadLine::ShuttingDown => {
                                self.write_response(
                                    421,
                                    format!("4.3.2 {} shutting down", self.params.hostname),
                                )
                                .await?;
                                return Ok(());
                            }
                        };
                        if line == "." {
                            break;
                        }

                        let line = if line.starts_with('.') {
                            &line[1..]
                        } else {
                            &line
                        };

                        data.extend_from_slice(line.as_bytes());
                        data.extend_from_slice(b"\r\n");
                    }

                    if too_long {
                        self.write_response(500, "5.2.3 line too long").await?;
                        continue;
                    }

                    let state = self
                        .state
                        .take()
                        .ok_or_else(|| anyhow!("transaction state is impossibly not set!?"))?;

                    tracing::trace!(?state);

                    let mut ids = vec![];
                    let mut messages = vec![];

                    let datestamp = Utc::now().to_rfc2822();

                    for recip in state.recipients {
                        let id = SpoolId::new();

                        let mut body = if self.params.trace_headers.received_header {
                            let received = {
                                let from_domain =
                                    self.said_hello.as_deref().unwrap_or("unspecified");
                                let peer_address = self.peer_address.ip();
                                let my_address = self.my_address.ip();
                                let hostname = &self.params.hostname;
                                let protocol = "ESMTP";
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
                            state.meta.clone(),
                            Arc::new(body.into_boxed_slice()),
                        )?;

                        if let Err(rej) = self
                            .call_callback("smtp_server_message_received", message.clone())
                            .await?
                        {
                            self.write_response(rej.code, rej.message).await?;
                            continue;
                        }

                        if self.params.trace_headers.supplemental_header {
                            let mut object = json!({
                                // Marker to identify encoded supplemental header
                                "_@_": "\\_/",
                                "recipient": message.recipient()?,
                            });

                            for name in &self.params.trace_headers.include_meta_names {
                                if let Ok(value) = message.get_meta(name) {
                                    object
                                        .as_object_mut()
                                        .unwrap()
                                        .insert(name.to_string(), value);
                                }
                            }

                            let value = base64::encode(serde_json::to_string(&object)?);
                            message.prepend_header(
                                Some(&self.params.trace_headers.header_name),
                                &value,
                            );
                        }

                        ids.push(message.id().to_string());

                        let queue_name = message.get_queue_name()?;

                        let relay_disposition =
                            self.check_relaying(&message.sender()?, &message.recipient()?);

                        if queue_name != "null" {
                            if relay_disposition.relay && !self.params.deferred_spool {
                                message.save().await?;
                            }
                            log_disposition(LogDisposition {
                                kind: RecordType::Reception,
                                msg: message.clone(),
                                site: "",
                                peer_address: Some(&ResolvedAddress {
                                    name: self.said_hello.as_deref().unwrap_or("").to_string(),
                                    addr: self.peer_address.ip(),
                                }),
                                response: Response {
                                    code: 250,
                                    enhanced_code: None,
                                    command: None,
                                    content: "".to_string(),
                                },
                                egress_pool: None,
                                egress_source: None,
                                relay_disposition: None,
                            })
                            .await;

                            if relay_disposition.relay {
                                messages.push((queue_name, message));
                            }
                        }
                    }

                    if !messages.is_empty() {
                        spawn_local(
                            format!(
                                "SmtpServer: insert {} msgs for {:?}",
                                messages.len(),
                                self.peer_address
                            ),
                            async move {
                                for (queue_name, msg) in messages {
                                    QueueManager::insert(&queue_name, msg).await?;
                                }
                                Ok::<(), anyhow::Error>(())
                            },
                        )?;
                    }

                    let ids = ids.join(" ");
                    self.write_response(250, format!("OK ids={ids}")).await?;
                }
                Ok(Command::Rset) => {
                    self.state.take();
                    self.write_response(250, "Reset state").await?;
                }
                Ok(Command::Noop(_)) => {
                    self.write_response(250, "the goggles do nothing").await?;
                }
                Ok(Command::Vrfy(_) | Command::Expn(_) | Command::Help(_)) => {
                    self.write_response(502, format!("5.5.1 Command unimplemented"))
                        .await?;
                }
            }
        }
    }
}

#[derive(Error, Debug)]
#[error("Error writing to client")]
struct WriteError;

const MAX_LINE_LEN: usize = 998;
#[derive(PartialEq)]
enum ReadLine {
    Line(String),
    TooLong,
    ShuttingDown,
    TimedOut,
    Disconnected,
}
