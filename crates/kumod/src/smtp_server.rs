use crate::lua_config::{load_config, LuaConfig};
use crate::queue::QueueManager;
use crate::spool::SpoolManager;
use anyhow::anyhow;
use cidr::IpCidr;
use message::{EnvelopeAddress, Message};
use mlua::ToLuaMulti;
use rfc5321::{AsyncReadAndWrite, BoxedAsyncReadAndWrite, Command};
use rustls::ServerConfig;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use tracing::{error, instrument};

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

#[derive(Debug)]
pub struct SmtpServer {
    socket: Option<BoxedAsyncReadAndWrite>,
    state: Option<TransactionState>,
    said_hello: Option<String>,
    config: LuaConfig,
    hostname: String,
    peer_address: SocketAddr,
    relay_hosts: Vec<IpCidr>,
    tls_active: bool,
    read_buffer: Vec<u8>,
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
        peer_address: SocketAddr,
        relay_hosts: Vec<IpCidr>,
        hostname: String,
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
            hostname,
            peer_address,
            relay_hosts,
            tls_active: false,
            read_buffer: Vec::with_capacity(1024),
        };

        tokio::task::spawn_local(async move {
            if let Err(err) = server.process().await {
                error!("Error in SmtpServer: {err:#}");
                server
                    .write_response(421, "4.3.0 technical difficulties")
                    .await
                    .ok();
            }
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
                format!("{} 4.3.2 Hold on just a moment!", self.hostname),
            )
            .await?;
            return Ok(());
        }

        self.write_response(220, format!("{} KumoMTA\nW00t!\nYeah!", self.hostname))
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
                    let acceptor = build_tls_acceptor(&self.hostname)?;
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
                            self.hostname,
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
                    for cidr in &self.relay_hosts {
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

                    let data = Arc::new(data.into_boxed_slice());

                    let mut ids = vec![];
                    let mut messages = vec![];

                    // TODO: allow selecting a different set of spools by defining
                    // a metadata key for that purpose
                    let data_spool = SpoolManager::get_named("data").await?;
                    let meta_spool = SpoolManager::get_named("meta").await?;

                    for recip in state.recipients {
                        let message = Message::new_dirty(
                            state.sender.clone(),
                            recip,
                            state.meta.clone(),
                            Arc::clone(&data),
                        )?;

                        if let Err(rej) = self
                            .call_callback("smtp_server_message_received", message.clone())
                            .await?
                        {
                            self.write_response(rej.code, rej.message).await?;
                            continue;
                        }

                        ids.push(message.id().to_string());

                        let queue_name = match message.get_meta_string("queue")? {
                            Some(name) => name.to_string(),
                            None => {
                                let campaign = message.get_meta_string("campaign")?;
                                let tenant = message.get_meta_string("tenant")?;
                                let domain = message.recipient()?.domain().to_string();
                                match (campaign, tenant) {
                                    (Some(c), Some(t)) => format!("{c}:{t}@{domain}"),
                                    (Some(c), None) => format!("{c}:@{domain}"),
                                    (None, Some(t)) => format!("{t}@{domain}"),
                                    (None, None) => domain,
                                }
                            }
                        };

                        if queue_name != "null" {
                            message
                                .save_to(&**meta_spool.lock().await, &**data_spool.lock().await)
                                .await?;
                            messages.push((queue_name, message));
                        }
                    }

                    let mut queue_manager = QueueManager::get().await;
                    for (queue_name, msg) in messages {
                        queue_manager.insert(&queue_name, msg).await?;
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

pub fn build_tls_acceptor(hostname: &str) -> anyhow::Result<TlsAcceptor> {
    let cert = rcgen::generate_simple_self_signed(vec![hostname.to_string()])?;
    let private_key = rustls::PrivateKey(cert.serialize_private_key_der());
    let cert = rustls::Certificate(cert.serialize_der()?);

    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], private_key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}
