use crate::lifecycle::{Activity, ShutdownSubcription};
use crate::logging::{log_disposition, RecordType};
use crate::mx::ResolvedAddress;
use crate::queue::QueueManager;
use crate::spool::SpoolManager;
use anyhow::{anyhow, Context};
use chrono::Utc;
use cidr::IpCidr;
use config::{load_config, LuaConfig};
use domain_map::DomainMap;
use message::{EnvelopeAddress, Message};
use mlua::ToLuaMulti;
use once_cell::sync::OnceCell;
use prometheus::IntGauge;
use rfc5321::{AsyncReadAndWrite, BoxedAsyncReadAndWrite, Command, ForwardPath, Response};
use rustls::ServerConfig;
use serde::Deserialize;
use spool::SpoolId;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, instrument};

#[derive(Deserialize, Clone, Debug, Default)]
pub struct EsmtpDomain {
    #[serde(default)]
    pub oob: bool,
    #[serde(default)]
    pub fbl: bool,
    #[serde(default)]
    pub relay: bool,
}

impl EsmtpDomain {
    pub fn accepts_rcptto(&self) -> bool {
        self.oob || self.fbl || self.relay
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct EsmtpListenerParams {
    #[serde(default = "EsmtpListenerParams::default_listen")]
    pub listen: String,
    #[serde(default = "EsmtpListenerParams::default_hostname")]
    pub hostname: String,
    #[serde(default = "EsmtpListenerParams::default_relay_hosts")]
    pub relay_hosts: Vec<IpCidr>,
    #[serde(default = "EsmtpListenerParams::default_banner")]
    pub banner: String,

    #[serde(default)]
    pub tls_certificate: Option<PathBuf>,
    #[serde(default)]
    pub tls_private_key: Option<PathBuf>,

    #[serde(default)]
    pub deferred_spool: bool,

    #[serde(default)]
    pub domains: DomainMap<EsmtpDomain>,

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

    fn default_relay_hosts() -> Vec<IpCidr> {
        vec![
            IpCidr::new("127.0.0.1".parse().unwrap(), 32).unwrap(),
            IpCidr::new("::1".parse().unwrap(), 128).unwrap(),
        ]
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

    pub fn build_tls_acceptor(&self) -> anyhow::Result<TlsAcceptor> {
        let config = self
            .tls_config
            .get_or_try_init(|| -> anyhow::Result<Arc<ServerConfig>> {
                crate::tls_helpers::make_server_config(
                    &self.hostname,
                    &self.tls_private_key,
                    &self.tls_certificate,
                )
            })?;

        Ok(TlsAcceptor::from(config.clone()))
    }

    pub fn connection_gauge(&self) -> &IntGauge {
        self.connection_gauge
            .get_or_init(|| crate::metrics_helper::connection_gauge_for_service("esmtp_listener"))
    }

    pub async fn run(self) -> anyhow::Result<()> {
        // Pre-create the acceptor so that we can share it across
        // the various listeners
        self.build_tls_acceptor()?;
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
                    crate::runtime::Runtime::run(move || {
                        tokio::task::spawn_local(async move {
                            if let Err(err) =
                                SmtpServer::run(socket, my_address, peer_address, params).await
                            {
                                tracing::error!("SmtpServer::run: {err:#}");
                            }
                        });
                    })
                    .await?;
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

impl SmtpServer {
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

        tokio::task::spawn_local(async move {
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
        });
        Ok(())
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

            let duration = std::time::Duration::from_secs(60);

            // Didn't find a complete line, fill up the rest of the buffer
            let mut data = [0u8; 1024];
            tokio::select! {
                _ = tokio::time::sleep(duration) => {
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
                    let acceptor = self.params.build_tls_acceptor()?;
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
                    let mut relay_allowed = false;
                    for cidr in &self.params.relay_hosts {
                        if cidr.contains(&self.peer_address.ip()) {
                            relay_allowed = true;
                            break;
                        }
                    }

                    match &address {
                        ForwardPath::Postmaster => {
                            // We must always accept <postmaster>
                            relay_allowed = true;
                        }
                        ForwardPath::Path(p) => {
                            if let Some(dom) =
                                self.params.domains.get(&p.mailbox.domain.to_string())
                            {
                                // Note that this can allow or deny depending
                                // on the config!
                                relay_allowed = dom.accepts_rcptto();
                            }
                        }
                    }

                    if !relay_allowed {
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
                    let address = EnvelopeAddress::parse(&address.to_string())?;
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
                        let received = {
                            let from_domain = self.said_hello.as_deref().unwrap_or("unspecified");
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

                        ids.push(message.id().to_string());

                        let queue_name = message.get_queue_name()?;

                        // Check destination domain prior to spooling;
                        // we may be set to process only OOB/FBL and
                        // not actually relay it
                        let recip = message.recipient()?;
                        let mut feedback_only = false;
                        if let Some(dom) = self.params.domains.get(recip.domain()) {
                            if !dom.relay {
                                feedback_only = true;
                            }
                        }

                        if queue_name != "null" {
                            if !feedback_only && !self.params.deferred_spool {
                                message.save().await?;
                            }
                            log_disposition(
                                RecordType::Reception,
                                message.clone(),
                                "",
                                Some(&ResolvedAddress {
                                    name: self.said_hello.as_deref().unwrap_or("").to_string(),
                                    addr: self.peer_address.ip(),
                                }),
                                Response {
                                    code: 250,
                                    enhanced_code: None,
                                    command: None,
                                    content: "".to_string(),
                                },
                                None,
                                None,
                            )
                            .await;

                            if !feedback_only {
                                messages.push((queue_name, message));
                            }
                        }
                    }

                    if !messages.is_empty() {
                        tokio::spawn(async move {
                            for (queue_name, msg) in messages {
                                QueueManager::insert(&queue_name, msg).await?;
                            }
                            Ok::<(), anyhow::Error>(())
                        });
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
