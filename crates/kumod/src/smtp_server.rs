use crate::queue::QueueManager;
use crate::spool::{SpoolHandle, SpoolManager};
use anyhow::{anyhow, Context};
use chrono::Utc;
use cidr::IpCidr;
use config::{load_config, LuaConfig};
use message::{EnvelopeAddress, Message};
use mlua::ToLuaMulti;
use once_cell::sync::OnceCell;
use prometheus::IntGauge;
use rfc5321::{AsyncReadAndWrite, BoxedAsyncReadAndWrite, Command};
use rustls::ServerConfig;
use serde::Deserialize;
use spool::SpoolId;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, instrument};

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

    #[serde(skip)]
    tls_config: OnceCell<Arc<ServerConfig>>,

    #[serde(skip)]
    connection_gauge: OnceCell<IntGauge>,
}

impl EsmtpListenerParams {
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
                let mut certificates = vec![];
                let private_key = match &self.tls_private_key {
                    Some(key) => load_private_key(key)?,
                    None => {
                        let cert =
                            rcgen::generate_simple_self_signed(vec![self.hostname.to_string()])?;
                        certificates.push(rustls::Certificate(cert.serialize_der()?));
                        rustls::PrivateKey(cert.serialize_private_key_der())
                    }
                };

                if let Some(cert_file) = &self.tls_certificate {
                    certificates = load_certs(cert_file)?;
                }

                let config = ServerConfig::builder()
                    .with_safe_defaults()
                    .with_no_client_auth()
                    .with_single_cert(certificates, private_key)?;

                Ok(Arc::new(config))
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

        loop {
            let (socket, peer_address) = listener.accept().await?;
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
    }
}

fn load_certs(filename: &Path) -> anyhow::Result<Vec<rustls::Certificate>> {
    let certfile = std::fs::File::open(filename)
        .with_context(|| format!("cannot open certificate file {}", filename.display()))?;

    let mut reader = std::io::BufReader::new(certfile);
    Ok(rustls_pemfile::certs(&mut reader)
        .with_context(|| {
            format!(
                "reading PEM encoded certificates from {}",
                filename.display()
            )
        })?
        .iter()
        .map(|v| rustls::Certificate(v.clone()))
        .collect())
}

fn load_private_key(filename: &Path) -> anyhow::Result<rustls::PrivateKey> {
    let keyfile = std::fs::File::open(filename)
        .with_context(|| format!("cannot open private key file {}", filename.display()))?;
    let mut reader = std::io::BufReader::new(keyfile);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::RSAKey(key)) => return Ok(rustls::PrivateKey(key)),
            Some(rustls_pemfile::Item::PKCS8Key(key)) => return Ok(rustls::PrivateKey(key)),
            Some(rustls_pemfile::Item::ECKey(key)) => return Ok(rustls::PrivateKey(key)),
            None => break,
            _ => {}
        }
    }

    anyhow::bail!(
        "no keys found in {} (encrypted keys not supported)",
        filename.display()
    );
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
    data_spool: SpoolHandle,
    meta_spool: SpoolHandle,
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
        let data_spool = SpoolManager::get_named("data").await?;
        let meta_spool = SpoolManager::get_named("meta").await?;
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
            data_spool,
            meta_spool,
        };

        tokio::task::spawn_local(async move {
            server.params.connection_gauge().inc();
            if let Err(err) = server.process().await {
                error!("Error in SmtpServer: {err:#}");
                server
                    .write_response(421, "4.3.0 technical difficulties")
                    .await
                    .ok();
            }
            server.params.connection_gauge().dec();
        });
        Ok(())
    }

    async fn write_response<S: AsRef<str>>(
        &mut self,
        status: u16,
        message: S,
    ) -> anyhow::Result<()> {
        if let Some(socket) = self.socket.as_mut() {
            let mut lines = message.as_ref().lines().peekable();
            while let Some(line) = lines.next() {
                let is_last = lines.peek().is_none();
                let sep = if is_last { ' ' } else { '-' };
                let text = format!("{status}{sep}{line}\r\n");
                socket.write(text.as_bytes()).await?;
            }
            socket.flush().await?;
        }
        Ok(())
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
            let size = self.socket.as_mut().unwrap().read(&mut data).await?;
            if size == 0 {
                anyhow::bail!("client disconnected");
            }
            self.read_buffer.extend_from_slice(&data[0..size]);
        }
    }

    pub async fn call_callback<'lua, S: AsRef<str>, A: ToLuaMulti<'lua> + Clone>(
        &'lua mut self,
        name: S,
        args: A,
    ) -> anyhow::Result<Result<(), RejectError>> {
        match self.config.async_call_callback(name, args).await {
            Ok(_) => Ok(Ok(())),
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
            let line = match self.read_line().await? {
                ReadLine::Line(line) => line,
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

                    let mut relay_allowed = false;
                    for cidr in &self.params.relay_hosts {
                        if cidr.contains(&self.peer_address.ip()) {
                            relay_allowed = true;
                            break;
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
                    if self.state.is_none() {
                        self.write_response(503, "5.5.0 MAIL FROM must be issued first")
                            .await?;
                        continue;
                    }
                    let address = EnvelopeAddress::parse(&address.to_string())?;
                    if let Err(rej) = self
                        .call_callback("smtp_server_mail_rcpt_to", address.clone())
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
                    self.write_response(354, "Send body; end with CRLF.CRLF")
                        .await?;

                    let mut data = vec![];
                    let mut too_long = false;

                    loop {
                        let line = match self.read_line().await? {
                            ReadLine::Line(line) => line,
                            ReadLine::TooLong => {
                                too_long = true;
                                data.clear();
                                continue;
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

                        if queue_name != "null" {
                            if !self.params.deferred_spool {
                                message
                                    .save_to(
                                        &**self.meta_spool.lock().await,
                                        &**self.data_spool.lock().await,
                                    )
                                    .await?;
                            }
                            messages.push((queue_name, message));
                        }
                    }

                    if !messages.is_empty() {
                        tokio::spawn(async move {
                            let mut queue_manager = QueueManager::get().await;
                            for (queue_name, msg) in messages {
                                queue_manager.insert(&queue_name, msg).await?;
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

const MAX_LINE_LEN: usize = 998;
#[derive(PartialEq)]
enum ReadLine {
    Line(String),
    TooLong,
}
