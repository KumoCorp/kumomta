#![allow(clippy::result_large_err)]
use crate::client_types::*;
use crate::parser::{Command, Domain, EsmtpParameter, ForwardPath, ReversePath};
use crate::{AsyncReadAndWrite, BoxedAsyncReadAndWrite};
use bstr::ByteSlice;
use hickory_proto::rr::rdata::TLSA;
use hickory_proto::rr::Name;
use memchr::memmem::Finder;
use nom_utils::DomainString;
use openssl::x509::{X509Ref, X509};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::time::{timeout, timeout_at, Instant};
use tokio_rustls::rustls::pki_types::ServerName;
use tracing::Level;

pub use kumo_tls_helper::TlsOptions;
pub use openssl;
pub use tokio_rustls;

const MAX_LINE_LEN: usize = 4096;

/// The message data is written to the peer in blocks of at most this
/// size, each subject to `SmtpClientTimeouts::data_block_timeout`.
/// RFC 5321 section 4.5.3.2.4 defines the data timeout in terms of each
/// block of data, rather than the transfer as a whole.
const DATA_BLOCK_SIZE: usize = 64 * 1024;

#[derive(Error, Debug, Clone)]
pub enum ClientError {
    #[error("response is not UTF8")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("Malformed Response: {0}")]
    MalformedResponseLine(String),
    #[error("Response line is too long")]
    ResponseTooLong,
    #[error("Not connected")]
    NotConnected,
    #[error("Command rejected {0:?}")]
    Rejected(Response),
    #[error("Commands rejected {0:?}")]
    RejectedBatch(Vec<Response>),
    #[error("STARTTLS: {0} is not a valid DNS name")]
    InvalidDnsName(String),
    #[error("Invalid client certificate configured: {error:?}")]
    FailedToBuildConnector { error: String },
    #[error("Timed Out waiting {duration:?} for response to cmd={}", self.command_for_err())]
    TimeOutResponse {
        command: Option<Command>,
        duration: Duration,
    },
    #[error("Timed Out after {duration:?} writing cmd={}", self.command_for_err())]
    TimeOutRequest {
        commands: Vec<Command>,
        duration: Duration,
    },
    #[error("Error {error} reading response to cmd={}", self.command_for_err())]
    ReadError {
        command: Option<Command>,
        error: String,
        partial: String,
    },
    #[error("Error {error} flushing send buffer")]
    FlushError { error: String },
    #[error("Error {error} writing {}", self.command_for_err())]
    WriteError {
        commands: Vec<Command>,
        error: String,
    },
    #[error("Timed Out sending message payload data")]
    TimeOutData,
    #[error("SSL Error: {0}")]
    SslErrorStack(#[from] openssl::error::ErrorStack),
    #[error("No usable DANE TLSA records for {hostname}: {tlsa:?}")]
    NoUsableDaneTlsa { hostname: String, tlsa: Vec<TLSA> },
}

impl ClientError {
    /// Returns the command(s) string suitable for passing into a Response,
    /// with any AUTH credential masked (see [`Command::encode_for_logging`]).
    pub fn command(&self) -> Option<String> {
        match self {
            Self::TimeOutResponse {
                command: Some(command),
                ..
            }
            | Self::ReadError {
                command: Some(command),
                ..
            } => Some(command.encode_for_logging().to_string()),
            Self::TimeOutRequest { commands, .. } | Self::WriteError { commands, .. }
                if !commands.is_empty() =>
            {
                let s: String = commands
                    .iter()
                    .map(|cmd| cmd.encode_for_logging().to_string())
                    .collect();
                Some(s)
            }
            _ => None,
        }
    }

    /// Returns the command(s) string suitable for embedding in the
    /// overall error message
    fn command_for_err(&self) -> String {
        self.command()
            .map(|cmd| cmd.replace("\r\n", ""))
            .unwrap_or_else(|| "NONE".to_string())
    }

    /// If the error contents were likely caused by something
    /// about the mostly recently attempted message, rather than
    /// a transport issue, or a carry-over from a prior message
    /// (eg: previous message was rejected and destination chose
    /// to drop the connection, which we detect later in RSET
    /// on the next message), then we return true.
    /// The expectation is that the caller will transiently
    /// fail the message for later retry.
    /// If we return false then the caller might decide to
    /// try that same message again more immediately on
    /// a separate connection
    pub fn was_due_to_message(&self) -> bool {
        match self {
            Self::Utf8(_)
            | Self::MalformedResponseLine(_)
            | Self::ResponseTooLong
            | Self::NotConnected
            | Self::InvalidDnsName(_)
            | Self::FailedToBuildConnector { .. }
            | Self::TimeOutResponse { .. }
            | Self::TimeOutRequest { .. }
            | Self::ReadError { .. }
            | Self::FlushError { .. }
            | Self::WriteError { .. }
            | Self::TimeOutData
            | Self::SslErrorStack(_)
            | Self::NoUsableDaneTlsa { .. } => false,
            Self::Rejected(response) => response.was_due_to_message(),
            Self::RejectedBatch(responses) => match responses.len() {
                1 => responses[0].was_due_to_message(),
                _ => false,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EsmtpCapability {
    pub name: String,
    pub param: Option<String>,
}

#[derive(Clone, Debug)]
pub enum SmtpClientTraceEvent {
    Closed,
    Read(Vec<u8>),
    Write(String),
    Diagnostic {
        level: tracing::Level,
        message: String,
    },
}

pub trait DeferredTracer {
    fn trace(&self) -> SmtpClientTraceEvent;
}

pub trait SmtpClientTracer: std::fmt::Debug {
    fn trace_event(&self, event: SmtpClientTraceEvent);
    fn lazy_trace(&self, deferred: &dyn DeferredTracer);
}

/// Receives a notification each time a block of message data has been
/// accepted by the peer, allowing the owner of the client to observe that
/// a long transfer is still making forward progress.
pub trait SmtpClientProgress: std::fmt::Debug + Send + Sync {
    fn progress(&self);
}

// helper to avoid making a second copy of every write buffer
struct WriteTracer<'a> {
    data: &'a str,
}
impl DeferredTracer for WriteTracer<'_> {
    fn trace(&self) -> SmtpClientTraceEvent {
        SmtpClientTraceEvent::Write(self.data.to_string())
    }
}
impl<'a> WriteTracer<'a> {
    fn trace(tracer: &Arc<dyn SmtpClientTracer + Send + Sync>, data: &'a str) {
        tracer.lazy_trace(&Self { data });
    }
}

struct BinWriteTracer<'a> {
    data: &'a [u8],
}
impl DeferredTracer for BinWriteTracer<'_> {
    fn trace(&self) -> SmtpClientTraceEvent {
        let data = String::from_utf8_lossy(self.data).to_string();
        SmtpClientTraceEvent::Write(data)
    }
}
impl<'a> BinWriteTracer<'a> {
    fn trace(tracer: &Arc<dyn SmtpClientTracer + Send + Sync>, data: &'a [u8]) {
        tracer.lazy_trace(&Self { data });
    }
}

// A little bit of gymnastics to avoid making a second
// copy of every read buffer
struct ReadTracer<'a> {
    data: &'a [u8],
}
impl DeferredTracer for ReadTracer<'_> {
    fn trace(&self) -> SmtpClientTraceEvent {
        SmtpClientTraceEvent::Read(self.data.to_vec())
    }
}

#[derive(Debug)]
pub struct SmtpClient {
    socket: Option<BoxedAsyncReadAndWrite>,
    hostname: String,
    capabilities: HashMap<String, EsmtpCapability>,
    read_buffer: Vec<u8>,
    timeouts: SmtpClientTimeouts,
    tracer: Option<Arc<dyn SmtpClientTracer + Send + Sync>>,
    progress: Option<Arc<dyn SmtpClientProgress>>,
    use_rset: bool,
    enable_rset: bool,
    enable_pipelining: bool,
    ignore_8bit_checks: bool,
}

fn extract_hostname(hostname: &str) -> &str {
    // Just the hostname, without any :port
    let fields: Vec<&str> = hostname.rsplitn(2, ':').collect();
    let hostname = if fields.len() == 2 {
        fields[1]
    } else {
        hostname
    };

    let hostname = if hostname.starts_with('[') && hostname.ends_with(']') {
        &hostname[1..hostname.len() - 1]
    } else {
        hostname
    };

    // Remove any trailing FQDN dot
    hostname.strip_suffix(".").unwrap_or(hostname)
}

impl SmtpClient {
    pub async fn new<A: ToSocketAddrs + ToString + Clone>(
        addr: A,
        timeouts: SmtpClientTimeouts,
    ) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr.clone()).await?;
        // No need for Nagle with SMTP request/response
        stream.set_nodelay(true)?;
        Ok(Self::with_stream(stream, addr.to_string(), timeouts))
    }

    pub fn with_stream<S: AsyncReadAndWrite + 'static, H: AsRef<str>>(
        stream: S,
        peer_hostname: H,
        timeouts: SmtpClientTimeouts,
    ) -> Self {
        let hostname = extract_hostname(peer_hostname.as_ref()).to_string();

        Self {
            socket: Some(Box::new(stream)),
            hostname,
            capabilities: HashMap::new(),
            read_buffer: Vec::with_capacity(1024),
            timeouts,
            tracer: None,
            progress: None,
            use_rset: false,
            enable_rset: false,
            enable_pipelining: false,
            ignore_8bit_checks: false,
        }
    }

    /// Setting this to true causes 8BITMIME and SMTPUTF8 checks to be ignored
    pub fn set_ignore_8bit_checks(&mut self, enable: bool) {
        self.ignore_8bit_checks = enable;
    }

    pub fn is_connected(&self) -> bool {
        self.socket.is_some()
    }

    pub fn set_enable_rset(&mut self, enable: bool) {
        self.enable_rset = enable;
    }

    pub fn set_enable_pipelining(&mut self, enable: bool) {
        self.enable_pipelining = enable;
    }

    pub fn set_tracer(&mut self, tracer: Arc<dyn SmtpClientTracer + Send + Sync>) {
        self.tracer.replace(tracer);
    }

    /// Register a notifier that is called as message data is written
    /// to the peer.
    pub fn set_progress_notifier(&mut self, progress: Arc<dyn SmtpClientProgress>) {
        self.progress.replace(progress);
    }

    pub fn timeouts(&self) -> &SmtpClientTimeouts {
        &self.timeouts
    }

    async fn read_line(
        &mut self,
        timeout_duration: Duration,
        cmd: Option<&Command>,
    ) -> Result<String, ClientError> {
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

                        if let Some(tracer) = &self.tracer {
                            tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                                level: Level::ERROR,
                                message: "Response too long".to_string(),
                            });
                        }

                        return Err(ClientError::ResponseTooLong);
                    }

                    let line = String::from_utf8(self.read_buffer[0..i].to_vec());
                    self.read_buffer.drain(0..i + 2);
                    return Ok(line?);
                }
            }
            if self.read_buffer.len() > MAX_LINE_LEN {
                self.read_buffer.clear();
                too_long = true;
            }

            // Didn't find a complete line, fill up the rest of the buffer
            let mut data = [0u8; MAX_LINE_LEN];
            let size = match self.socket.as_mut() {
                Some(s) => match timeout(timeout_duration, s.read(&mut data)).await {
                    Ok(Ok(size)) => size,
                    Ok(Err(err)) => {
                        self.socket.take();
                        if let Some(tracer) = &self.tracer {
                            tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                                level: Level::ERROR,
                                message: format!("Error during read: {err:#}"),
                            });
                            tracer.trace_event(SmtpClientTraceEvent::Closed);
                        }
                        return Err(ClientError::ReadError {
                            command: cmd.cloned(),
                            error: format!("{err:#}"),
                            partial: String::from_utf8_lossy(&self.read_buffer).to_string(),
                        });
                    }
                    Err(_) => {
                        // If we're using a zero timeout we're probing
                        // for liveness, so we don't want to treat this
                        // timeout as a fatal error that closes the
                        // connection
                        if timeout_duration != Duration::ZERO {
                            self.socket.take();
                        }
                        if let Some(tracer) = &self.tracer {
                            tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                                level: Level::ERROR,
                                message: format!("Read Timeout after {timeout_duration:?}"),
                            });
                            if self.socket.is_none() {
                                tracer.trace_event(SmtpClientTraceEvent::Closed);
                            }
                        }
                        return Err(ClientError::TimeOutResponse {
                            command: cmd.cloned(),
                            duration: timeout_duration,
                        });
                    }
                },
                None => {
                    return Err(ClientError::ReadError {
                        command: cmd.cloned(),
                        error: "the socket was closed in response to an earlier issue".to_string(),
                        partial: String::from_utf8_lossy(&self.read_buffer).to_string(),
                    });
                }
            };
            if size == 0 {
                self.socket.take();
                if let Some(tracer) = &self.tracer {
                    tracer.trace_event(SmtpClientTraceEvent::Closed);
                }
                return Err(ClientError::ReadError {
                    command: cmd.cloned(),
                    error: "Connection closed by peer".to_string(),
                    partial: String::from_utf8_lossy(&self.read_buffer).to_string(),
                });
            }
            if let Some(tracer) = &self.tracer {
                tracer.lazy_trace(&ReadTracer {
                    data: &data[0..size],
                });
            }
            self.read_buffer.extend_from_slice(&data[0..size]);
        }
    }

    /// Check to see if there is either a unilateral response
    /// or an error condition immediately available on the socket.
    ///
    /// Intended to be used after the connection has idled to see if
    /// the peer has opted to close the connection, either gracefully
    /// by sending what should be a 421, or abruptly by simply snipping
    /// the connection.
    pub async fn check_unilateral_response(&mut self) -> Result<Option<Response>, ClientError> {
        match self.read_response(None, Duration::ZERO).await {
            // Unilateral response was received
            Ok(response) => Ok(Some(response)),
            // Nothing immediately available: this should hopefully be the common case
            Err(ClientError::TimeOutResponse { .. }) => Ok(None),
            // An actual error
            Err(err) => Err(err),
        }
    }

    pub async fn read_response(
        &mut self,
        command: Option<&Command>,
        timeout_duration: Duration,
    ) -> Result<Response, ClientError> {
        let deadline = Instant::now() + timeout_duration;

        if let Some(sock) = self.socket.as_mut() {
            match timeout_at(deadline, sock.flush()).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    self.socket.take();
                    if let Some(tracer) = &self.tracer {
                        tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                            level: Level::ERROR,
                            message: format!("Error during flush: {err:#}"),
                        });
                        tracer.trace_event(SmtpClientTraceEvent::Closed);
                    }
                    return Err(ClientError::FlushError {
                        error: format!("{err:#}"),
                    });
                }
                Err(_elapsed) => {
                    self.socket.take();
                    if let Some(tracer) = &self.tracer {
                        tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                            level: Level::ERROR,
                            message: format!("Flush timed out after {timeout_duration:?}"),
                        });
                        tracer.trace_event(SmtpClientTraceEvent::Closed);
                    }
                    return Err(ClientError::TimeOutResponse {
                        command: command.cloned(),
                        duration: timeout_duration,
                    });
                }
            }
        }

        let mut line = self
            .read_line(deadline.saturating_duration_since(Instant::now()), command)
            .await?;
        tracing::trace!("recv<-{}: {line}", self.hostname);
        let mut parsed = parse_response_line(&line)?;
        let mut response_builder = ResponseBuilder::new(&parsed);

        while !parsed.is_final {
            let per_line =
                Duration::from_secs(60).min(deadline.saturating_duration_since(Instant::now()));
            line = self.read_line(per_line, command).await?;
            parsed = parse_response_line(&line)?;
            response_builder
                .add_line(&parsed)
                .map_err(ClientError::MalformedResponseLine)?;
        }

        let response =
            response_builder.build(command.map(|cmd| cmd.encode_for_logging().to_string()));

        tracing::trace!("{}: {response:?}", self.hostname);

        Ok(response)
    }

    pub async fn send_command(&mut self, command: &Command) -> Result<Response, ClientError> {
        self.write_command_request(command).await?;
        self.read_response(Some(command), command.client_timeout(&self.timeouts))
            .await
    }

    /// Wrapper around socket.write_all() that will emit trace diagnostics and synthesize
    /// a Close event to the tracer if a timeout or IO error occurs.
    /// If an error or timeout, occurs ensures that the socket will not be reused.
    async fn write_all_with_timeout<F, G>(
        &mut self,
        timeout_duration: Duration,
        bytes: &[u8],
        make_timeout_err: F,
        make_write_err: G,
    ) -> Result<(), ClientError>
    where
        F: FnOnce() -> ClientError,
        G: FnOnce(String) -> ClientError,
    {
        match self.socket.as_mut() {
            Some(socket) => match timeout(timeout_duration, socket.write_all(bytes)).await {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(err)) => {
                    self.socket.take();
                    if let Some(tracer) = &self.tracer {
                        tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                            level: Level::ERROR,
                            message: format!("Error during write: {err:#}"),
                        });
                        tracer.trace_event(SmtpClientTraceEvent::Closed);
                    }
                    Err(make_write_err(format!("{err:#}")))
                }
                Err(_) => {
                    self.socket.take();
                    if let Some(tracer) = &self.tracer {
                        tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                            level: Level::ERROR,
                            message: format!("Write Timeout after {timeout_duration:?}"),
                        });
                        tracer.trace_event(SmtpClientTraceEvent::Closed);
                    }
                    Err(make_timeout_err())
                }
            },
            None => Err(make_write_err(
                "the socket was closed in response to an earlier issue".to_string(),
            )),
        }
    }

    async fn write_pipeline_request(&mut self, commands: &[Command]) -> Result<(), ClientError> {
        let total_timeout: Duration = commands
            .iter()
            .map(|cmd| cmd.client_timeout_request(&self.timeouts))
            .sum();

        let mut lines: Vec<String> = vec![];
        let mut all: Vec<u8> = vec![];
        for cmd in commands {
            let line = cmd.encode();
            all.extend_from_slice(&line);
            lines.push(line.to_string());
        }
        tracing::trace!(
            "send->{}: (PIPELINE) {}",
            self.hostname,
            all.as_bstr().escape_bytes()
        );
        if self.socket.is_some() {
            if let Some(tracer) = &self.tracer {
                // Send the lines individually to the tracer, so that we
                // don't break --terse mode
                for line in &lines {
                    WriteTracer::trace(tracer, line);
                }
            }
        }
        self.write_all_with_timeout(
            total_timeout,
            &all,
            || ClientError::TimeOutRequest {
                duration: total_timeout,
                commands: commands.to_vec(),
            },
            |error| ClientError::WriteError {
                error,
                commands: commands.to_vec(),
            },
        )
        .await
    }

    async fn write_command_request(&mut self, command: &Command) -> Result<(), ClientError> {
        let line = command.encode();
        tracing::trace!("send->{}: {line}", self.hostname);
        if self.socket.is_some() {
            if let Some(tracer) = &self.tracer {
                WriteTracer::trace(tracer, &line.to_string());
            }
        }

        let timeout_duration = command.client_timeout_request(&self.timeouts);
        self.write_all_with_timeout(
            timeout_duration,
            &line,
            || ClientError::TimeOutRequest {
                duration: timeout_duration,
                commands: vec![command.clone()],
            },
            |error| ClientError::WriteError {
                error,
                commands: vec![command.clone()],
            },
        )
        .await
    }

    async fn write_data_with_timeout(&mut self, data: &[u8]) -> Result<(), ClientError> {
        if self.socket.is_some() {
            if let Some(tracer) = &self.tracer {
                BinWriteTracer::trace(tracer, data);
            }
        }
        // Write in blocks, each with its own deadline, so that a slow
        // but steady peer can accept a large message without the overall
        // transfer time being limited.
        let timeout_duration = self.timeouts.data_block_timeout;
        for block in data.chunks(DATA_BLOCK_SIZE) {
            self.write_all_with_timeout(
                timeout_duration,
                block,
                || ClientError::TimeOutData,
                |error| ClientError::WriteError {
                    error,
                    commands: vec![],
                },
            )
            .await?;
            if let Some(progress) = &self.progress {
                progress.progress();
            }
        }
        Ok(())
    }

    /// Issue a series of commands, and return the responses to
    /// those commands.
    ///
    /// If the server advertised the RFC 2920 PIPELINING extension,
    /// the commands are written one after the other before waiting
    /// to read any data, resulting in lower overall latency due
    /// to round-trip-times.
    ///
    /// If PIPELINING is not available, each command is written
    /// and the response read before attempting to write the next
    /// command.
    ///
    /// The number of returned responses may be smaller than the
    /// number of requested commands if there is an issue with
    /// the network connection.
    pub async fn pipeline_commands(
        &mut self,
        commands: Vec<Command>,
    ) -> Vec<Result<Response, ClientError>> {
        let mut results: Vec<Result<Response, ClientError>> = vec![];

        let pipeline = self.enable_pipelining && self.capabilities.contains_key("PIPELINING");
        if pipeline {
            if let Err(err) = self.write_pipeline_request(&commands).await {
                let err: ClientError = err;
                results.push(Err(err.clone()));
                while results.len() < commands.len() {
                    // Synthesize failures for the remaining commands
                    results.push(Err(err.clone()));
                }
                return results;
            }

            // Now read the responses effectively in a batch
            for cmd in &commands {
                results.push(
                    self.read_response(Some(cmd), cmd.client_timeout(&self.timeouts))
                        .await,
                );
            }
            return results;
        }

        for cmd in &commands {
            if let Err(err) = self.write_command_request(cmd).await {
                let err: ClientError = err;
                results.push(Err(err.clone()));
                while results.len() < commands.len() {
                    // Synthesize failures for the remaining commands
                    results.push(Err(err.clone()));
                }
                return results;
            }
            // Immediately request the response if the server
            // doesn't support pipelining
            results.push(
                self.read_response(Some(cmd), cmd.client_timeout(&self.timeouts))
                    .await,
            );
        }
        results
    }

    pub async fn ehlo_lhlo(
        &mut self,
        ehlo_name: &str,
        use_lmtp: bool,
    ) -> Result<&HashMap<String, EsmtpCapability>, ClientError> {
        if use_lmtp {
            self.lhlo(ehlo_name).await
        } else {
            self.ehlo(ehlo_name).await
        }
    }

    pub async fn lhlo(
        &mut self,
        ehlo_name: &str,
    ) -> Result<&HashMap<String, EsmtpCapability>, ClientError> {
        let response = self
            .send_command(&Command::Lhlo(Domain::DomainName(
                ehlo_name
                    .parse::<DomainString>()
                    .map_err(|_| ClientError::InvalidDnsName(ehlo_name.to_string()))?,
            )))
            .await?;
        self.ehlo_common(response)
    }

    pub async fn ehlo(
        &mut self,
        ehlo_name: &str,
    ) -> Result<&HashMap<String, EsmtpCapability>, ClientError> {
        let response = self
            .send_command(&Command::Ehlo(Domain::DomainName(
                ehlo_name
                    .parse::<DomainString>()
                    .map_err(|_| ClientError::InvalidDnsName(ehlo_name.to_string()))?,
            )))
            .await?;
        self.ehlo_common(response)
    }

    fn ehlo_common(
        &mut self,
        response: Response,
    ) -> Result<&HashMap<String, EsmtpCapability>, ClientError> {
        if response.code != 250 {
            return Err(ClientError::Rejected(response));
        }

        let mut capabilities = HashMap::new();

        for line in response.content.lines().skip(1) {
            let mut fields = line.splitn(2, ' ');
            if let Some(name) = fields.next() {
                let param = fields.next().map(|s| s.to_string());
                let cap = EsmtpCapability {
                    name: name.to_string(),
                    param,
                };
                capabilities.insert(name.to_ascii_uppercase(), cap);
            }
        }

        self.capabilities = capabilities;
        Ok(&self.capabilities)
    }

    pub async fn auth_plain(
        &mut self,
        username: &str,
        password: Option<&str>,
    ) -> Result<(), ClientError> {
        // RFC 4616 says that the format is:
        // [authzid] NUL authcid NUL passwd
        let password = password.unwrap_or("");
        let payload = format!("\x00{username}\x00{password}");
        let payload = data_encoding::BASE64.encode(payload.as_bytes());

        let response = self
            .send_command(&Command::Auth {
                sasl_mech: "PLAIN".to_string(),
                initial_response: Some(payload),
            })
            .await?;

        if response.code != 235 {
            return Err(ClientError::Rejected(response));
        }

        Ok(())
    }

    /// Send the STARTTLS command and, if the server accepts it, perform the TLS
    /// handshake to upgrade the connection.
    ///
    /// Returns Err if the server rejects STARTTLS or a transport error occurs.
    /// A failed handshake is instead reported as
    /// `Ok(TlsStatus::FailedHandshake(_))` rather than Err. On success the
    /// socket is upgraded to TLS and the client remains usable. On a failed
    /// handshake the client is unusable. A new connection is required to send
    /// further commands.
    pub async fn starttls(&mut self, options: TlsOptions) -> Result<TlsStatus, ClientError> {
        let resp = self.send_command(&Command::StartTls).await?;
        if resp.code != 220 {
            return Err(ClientError::Rejected(resp));
        }

        self.tls_handshake(options).await
    }

    /// Perform the TLS handshake directly on the current socket, without a
    /// preceding STARTTLS negotiation. Used for implicit TLS (SMTPS), where
    /// the session is encrypted from the first byte.
    pub async fn tls_connect(&mut self, options: TlsOptions) -> Result<TlsStatus, ClientError> {
        self.tls_handshake(options).await
    }

    async fn tls_handshake(&mut self, options: TlsOptions) -> Result<TlsStatus, ClientError> {
        let mut handshake_error = None;
        let mut tls_info = TlsInformation::default();

        let stream: Option<BoxedAsyncReadAndWrite> = if options.prefer_openssl
            || !options.dane_tlsa.is_empty()
        {
            let connector = options
                .build_openssl_connector(&self.hostname)
                .map_err(|error| ClientError::FailedToBuildConnector {
                    error: error.to_string(),
                })?;
            let ssl = connector.into_ssl(self.hostname.as_str())?;

            let stream = match self.socket.take() {
                Some(s) => s,
                None => return Err(ClientError::NotConnected),
            };

            let mut ssl_stream = tokio_openssl::SslStream::new(ssl, stream)?;

            match timeout(
                self.timeouts.starttls_timeout,
                std::pin::Pin::new(&mut ssl_stream).connect(),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    handshake_error.replace(format!("{err:#}"));
                }
                Err(_elapsed) => {
                    if let Some(tracer) = &self.tracer {
                        tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                            level: Level::ERROR,
                            message: format!(
                                "openssl handshake timed out after {:?}",
                                self.timeouts.starttls_timeout
                            ),
                        });
                        tracer.trace_event(SmtpClientTraceEvent::Closed);
                    }
                    return Err(ClientError::TimeOutResponse {
                        command: Some(Command::StartTls),
                        duration: self.timeouts.starttls_timeout,
                    });
                }
            }

            tls_info.provider_name = "openssl".to_string();
            tls_info.cipher = match ssl_stream.ssl().current_cipher() {
                Some(cipher) => cipher.standard_name().unwrap_or(cipher.name()).to_string(),
                None => String::new(),
            };
            tls_info.protocol_version = ssl_stream.ssl().version_str().to_string();

            if let Some(cert) = ssl_stream.ssl().peer_certificate() {
                tls_info.subject_name = subject_name(&cert);
            }
            if let Ok(authority) = ssl_stream.ssl().dane_authority() {
                if let Some(cert) = &authority.cert {
                    tls_info.subject_name = subject_name(cert);
                }
            }

            if handshake_error.is_some() {
                None
            } else {
                Some(Box::new(ssl_stream) as BoxedAsyncReadAndWrite)
            }
        } else {
            tls_info.provider_name = "rustls".to_string();
            let connector = options.build_tls_connector().await.map_err(|error| {
                ClientError::FailedToBuildConnector {
                    error: error.to_string(),
                }
            })?;
            let server_name = parse_server_name(self.hostname.as_str())?;

            let socket = match self.socket.take() {
                Some(s) => s,
                None => return Err(ClientError::NotConnected),
            };

            let connect_future = connector.connect(server_name, socket);
            match timeout(self.timeouts.starttls_timeout, connect_future).await {
                Ok(Ok(stream)) => {
                    let (_, conn) = stream.get_ref();
                    tls_info.cipher = match conn.negotiated_cipher_suite() {
                        Some(suite) => suite.suite().as_str().unwrap_or("UNKNOWN").to_string(),
                        None => String::new(),
                    };
                    tls_info.protocol_version = match conn.protocol_version() {
                        Some(version) => version.as_str().unwrap_or("UNKNOWN").to_string(),
                        None => String::new(),
                    };

                    if let Some(certs) = conn.peer_certificates() {
                        let peer_cert = &certs[0];
                        if let Ok(cert) = X509::from_der(peer_cert.as_ref()) {
                            tls_info.subject_name = subject_name(&cert);
                        }
                    }

                    Some(Box::new(stream) as BoxedAsyncReadAndWrite)
                }
                Ok(Err(err)) => {
                    handshake_error.replace(format!("{err:#}"));
                    None
                }
                Err(_elapsed) => {
                    if let Some(tracer) = &self.tracer {
                        tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                            level: Level::ERROR,
                            message: format!(
                                "rustls handshake timed out after {:?}",
                                self.timeouts.starttls_timeout
                            ),
                        });
                        tracer.trace_event(SmtpClientTraceEvent::Closed);
                    }
                    return Err(ClientError::TimeOutResponse {
                        command: Some(Command::StartTls),
                        duration: self.timeouts.starttls_timeout,
                    });
                }
            }
        };

        // On a successful handshake the peer was authenticated iff verification
        // was required: when `insecure` is set we accept the peer's certificate
        // without validating it, so the session is encrypted but unauthenticated.
        tls_info.authenticated = !options.insecure;

        if let Some(tracer) = &self.tracer {
            tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                level: Level::INFO,
                message: match &handshake_error {
                    Some(error) => format!("TLS handshake failed: {error:?}"),
                    None => format!("TLS handshake -> {tls_info:?}"),
                },
            });
        }

        if let Some(stream) = stream {
            self.socket.replace(stream);
        }
        Ok(match handshake_error {
            Some(error) => TlsStatus::FailedHandshake(error),
            None => TlsStatus::Info(tls_info),
        })
    }

    pub async fn send_mail<B: AsRef<[u8]>, SENDER: Into<ReversePath>, RECIP: Into<ForwardPath>>(
        &mut self,
        sender: SENDER,
        recipient: RECIP,
        data: B,
    ) -> Result<Response, ClientError> {
        let recipient: ForwardPath = recipient.into();
        let status = self
            .send_mail_multi_recip(sender, vec![recipient], data)
            .await?;
        Ok(status.response)
    }

    pub async fn send_mail_multi_recip<B: AsRef<[u8]>, SENDER: Into<ReversePath>>(
        &mut self,
        sender: SENDER,
        recipient_list: Vec<ForwardPath>,
        data: B,
    ) -> Result<BatchSendSuccess, ClientError> {
        let sender = sender.into();

        let data: &[u8] = data.as_ref();
        let stuffed;

        let data = match apply_dot_stuffing(data) {
            Some(d) => {
                stuffed = d;
                &stuffed
            }
            None => data,
        };

        let data_is_8bit = data.iter().any(|&b| b >= 0x80);
        let envelope_is_8bit =
            !sender.is_ascii() || recipient_list.iter().any(|recipient| !recipient.is_ascii());

        let mut mail_from_params = vec![];
        if data_is_8bit {
            if self.capabilities.contains_key("8BITMIME") {
                mail_from_params.push(EsmtpParameter {
                    name: "BODY".to_string(),
                    value: Some("8BITMIME".to_string()),
                });
            } else if !self.ignore_8bit_checks {
                return Err(ClientError::Rejected(Response {
                    code: 554,
                    command: None,
                    enhanced_code: Some(EnhancedStatusCode {
                        class: 5,
                        subject: 6,
                        detail: 3,
                    }),
                    content: "KumoMTA internal: DATA is 8bit, destination does \
                        not support 8BITMIME. Conversion via msg:check_fix_conformance \
                        during reception is required"
                        .to_string(),
                }));
            }
        }

        if envelope_is_8bit {
            if self.capabilities.contains_key("SMTPUTF8") {
                mail_from_params.push(EsmtpParameter {
                    name: "SMTPUTF8".to_string(),
                    value: None,
                });
            } else if !self.ignore_8bit_checks {
                return Err(ClientError::Rejected(Response {
                    code: 554,
                    command: None,
                    enhanced_code: Some(EnhancedStatusCode {
                        class: 5,
                        subject: 6,
                        detail: 7,
                    }),
                    content: "KumoMTA internal: envelope is 8bit, destination does \
                        not support SMTPUTF8."
                        .to_string(),
                }));
            }
        }

        let mut commands = vec![];

        // We want to avoid using RSET for the first message we send on
        // a given connection, because postfix can run in a mode where
        // it will not tolerate RSET because it considers it to be a "junk"
        // command, and rejecting junk commands will cut down on its load
        // when it is under stress; it is used as a load shedding approach.
        // If we always RSET then we will never deliver to a site that is
        // configured that way. If we take care to RSET only for subsequent
        // sends, then we should get one message per connection through
        // without being unfairly penalized for defensively RSETing.
        let used_rset = self.use_rset;
        if self.use_rset {
            commands.push(Command::Rset);
        }
        commands.push(Command::MailFrom {
            address: sender,
            parameters: mail_from_params,
        });

        for recipient in &recipient_list {
            commands.push(Command::RcptTo {
                address: recipient.clone(),
                parameters: vec![],
            });
        }
        commands.push(Command::Data);

        // Assume that something might break below: if it does, we want
        // to ensure that we RSET the connection on the next go around.
        self.use_rset = true;

        let mut responses = self.pipeline_commands(commands).await;

        // This is a little awkward. We want to handle the RFC 2090 3.1 case
        // below, which requires deferring checking the actual response codes
        // until later, but we must also handle the case where we had a hard
        // transport error partway through pipelining.
        // So we set a flag for that case and will then "eagerly", wrt. the
        // RFC 2090 3.1 logic, evaluate the SMTP response codes, so that we
        // can propagate the correct error disposition up to the caller.
        let is_err = responses.iter().any(|r| r.is_err());

        if used_rset {
            let rset_resp = responses.remove(0)?;
            if rset_resp.code != 250 {
                return Err(ClientError::Rejected(rset_resp));
            }
        }

        let mail_resp = responses.remove(0)?;
        if is_err && mail_resp.code != 250 {
            return Err(ClientError::Rejected(mail_resp));
        }

        let mut rcpt_responses = vec![];
        for _ in &recipient_list {
            rcpt_responses.push(responses.remove(0)?);
        }

        if is_err && rcpt_responses.iter().all(|resp| resp.code != 250) {
            return Err(ClientError::RejectedBatch(rcpt_responses));
        }

        let data_resp = responses.remove(0)?;
        if is_err && data_resp.code != 354 {
            return Err(ClientError::Rejected(data_resp));
        }

        if data_resp.code == 354
            && (mail_resp.code != 250 || rcpt_responses.iter().all(|resp| resp.code != 250))
        {
            // RFC 2920 3.1:
            // the client cannot assume that the DATA command will be rejected
            // just because none of the RCPT TO commands worked.  If the DATA
            // command was properly rejected the client SMTP can just issue
            // RSET, but if the DATA command was accepted the client SMTP
            // should send a single dot.

            // Send dummy data
            self.write_data_with_timeout(b".\r\n").await?;
            let data_dot = Command::DataDot;
            // wait for its response
            let _ = self
                .read_response(Some(&data_dot), data_dot.client_timeout(&self.timeouts))
                .await?;

            // Continue below: we will match one of the failure cases and
            // return a ClientError::Rejected from one of the earlier
            // commands
        }

        if mail_resp.code != 250 {
            return Err(ClientError::Rejected(mail_resp));
        }
        if rcpt_responses.iter().all(|resp| resp.code != 250) {
            if rcpt_responses.len() == 1 {
                return Err(ClientError::Rejected(
                    rcpt_responses.pop().expect("have at least one"),
                ));
            }
            return Err(ClientError::RejectedBatch(rcpt_responses));
        }
        if data_resp.code != 354 {
            return Err(ClientError::Rejected(data_resp));
        }

        let needs_newline = data.last().map(|&b| b != b'\n').unwrap_or(true);

        tracing::trace!("message data is {} bytes", data.len());

        self.write_data_with_timeout(data).await?;

        let marker = if needs_newline { "\r\n.\r\n" } else { ".\r\n" };

        tracing::trace!("send->{}: {}", self.hostname, marker.escape_debug());

        self.write_data_with_timeout(marker.as_bytes()).await?;

        let data_dot = Command::DataDot;
        let resp = self
            .read_response(Some(&data_dot), data_dot.client_timeout(&self.timeouts))
            .await?;
        if resp.code != 250 {
            return Err(ClientError::Rejected(resp));
        }

        // If everything went well, respect the user preference for speculatively
        // issuing an RSET next time around
        self.use_rset = self.enable_rset;

        Ok(BatchSendSuccess {
            response: resp,
            rcpt_responses,
        })
    }
}

#[derive(Debug)]
pub struct BatchSendSuccess {
    pub response: Response,
    pub rcpt_responses: Vec<Response>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub enum TlsStatus {
    FailedHandshake(String),
    Info(TlsInformation),
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct TlsInformation {
    pub cipher: String,
    pub protocol_version: String,
    pub subject_name: Vec<String>,
    pub provider_name: String,
    /// Whether the peer certificate was authenticated (validated) as part of
    /// the handshake. False when the session is encrypted but the peer was not
    /// verified (e.g. opportunistic-insecure, or DANE with no usable TLSA
    /// records).
    pub authenticated: bool,
}

impl Drop for SmtpClient {
    fn drop(&mut self) {
        if let Some(tracer) = &self.tracer {
            if self.socket.is_some() {
                tracer.trace_event(SmtpClientTraceEvent::Closed);
            }
        }
    }
}
fn parse_response_line(line: &'_ str) -> Result<ResponseLine<'_>, ClientError> {
    if line.len() < 4 {
        return Err(ClientError::MalformedResponseLine(line.to_string()));
    }

    match line.as_bytes()[3] {
        b' ' | b'-' => match line[0..3].parse::<u16>() {
            Ok(code) => Ok(ResponseLine {
                code,
                is_final: line.as_bytes()[3] == b' ',
                content: &line[4..],
            }),
            Err(_) => Err(ClientError::MalformedResponseLine(line.to_string())),
        },
        _ => Err(ClientError::MalformedResponseLine(line.to_string())),
    }
}

fn apply_dot_stuffing(data: &[u8]) -> Option<Vec<u8>> {
    static LFDOT: LazyLock<Finder> = LazyLock::new(|| memchr::memmem::Finder::new("\n."));

    if !data.starts_with(b".") && LFDOT.find(data).is_none() {
        return None;
    }

    let mut stuffed = vec![];
    if data.starts_with(b".") {
        stuffed.push(b'.');
    }
    let mut last_idx = 0;
    for i in LFDOT.find_iter(data) {
        stuffed.extend_from_slice(&data[last_idx..=i]);
        stuffed.push(b'.');
        last_idx = i + 1;
    }
    stuffed.extend_from_slice(&data[last_idx..]);
    Some(stuffed)
}

/// Extracts the object=name pairs of the subject name from a cert.
/// eg:
/// ```text
/// ["C=US", "ST=CA", "L=SanFrancisco", "O=Fort-Funston", "OU=MyOrganizationalUnit",
/// "CN=do.havedane.net", "name=EasyRSA", "emailAddress=me@myhost.mydomain"]
/// ```
pub fn subject_name(cert: &X509Ref) -> Vec<String> {
    let mut subject_name = vec![];
    for entry in cert.subject_name().entries() {
        if let Ok(obj) = entry.object().nid().short_name() {
            if let Ok(name) = entry.data().as_utf8() {
                subject_name.push(format!("{obj}={name}"));
            }
        }
    }
    subject_name
}

fn parse_server_name(input: &str) -> Result<ServerName<'static>, ClientError> {
    match IpAddr::from_str(input) {
        Ok(ip) => Ok(ServerName::IpAddress(ip.into())),
        Err(_) => {
            let name = Name::from_str_relaxed(input)
                .map_err(|_| ClientError::InvalidDnsName(input.to_string()))?;
            ServerName::try_from(name.to_ascii())
                .map_err(|_| ClientError::InvalidDnsName(name.to_ascii()))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::parser::{EnvelopeAddress, ReversePath};

    #[test]
    fn test_stuffing() {
        assert_eq!(apply_dot_stuffing(b"foo"), None);
        assert_eq!(apply_dot_stuffing(b".foo").unwrap(), b"..foo");
        assert_eq!(apply_dot_stuffing(b"foo\n.bar").unwrap(), b"foo\n..bar");
        assert_eq!(
            apply_dot_stuffing(b"foo\n.bar\n..baz\n").unwrap(),
            b"foo\n..bar\n...baz\n"
        );
    }

    /*
    #[tokio::test]
    async fn test_against_sink() {
        use tokio::net::TcpStream;
        let stream = TcpStream::connect("127.0.0.1:2025").await.unwrap();
        let mut client =
            SmtpClient::with_stream(stream, "localhost", SmtpClientTimeouts::default());
        dbg!(client.read_response(None).await).unwrap();
        dbg!(client.ehlo("localhost").await).unwrap();
        let insecure = true;
        dbg!(client.starttls(insecure).await).unwrap();
        let resp = client
            .send_mail(
                ReversePath::try_from("wez@mail.example.com").unwrap(),
                ForwardPath::try_from("wez@mail.example.com").unwrap(),
                "Subject: hello\r\n\r\nwoot\r\n",
            )
            .await
            .unwrap();
        panic!("{resp:#?}");
    }
    */

    #[test]
    fn response_line_parsing() {
        assert_eq!(
            parse_response_line("220 woot").unwrap(),
            ResponseLine {
                code: 220,
                is_final: true,
                content: "woot"
            }
        );
        assert_eq!(
            parse_response_line("220-woot").unwrap(),
            ResponseLine {
                code: 220,
                is_final: false,
                content: "woot"
            }
        );

        assert!(matches!(
            parse_response_line("220_woot"),
            Err(ClientError::MalformedResponseLine(_))
        ));
        assert!(matches!(
            parse_response_line("not really"),
            Err(ClientError::MalformedResponseLine(_))
        ));
    }

    fn parse_multi_line(lines: &[&str]) -> Result<Response, ClientError> {
        let mut parsed = parse_response_line(lines[0])?;
        let mut b = ResponseBuilder::new(&parsed);
        for line in &lines[1..] {
            parsed = parse_response_line(line)?;
            b.add_line(&parsed)
                .map_err(ClientError::MalformedResponseLine)?;
        }
        assert!(parsed.is_final);
        Ok(b.build(None))
    }

    #[test]
    fn multi_line_response() {
        assert_eq!(
            parse_multi_line(&["220-woot", "220-more", "220 done",]).unwrap(),
            Response {
                code: 220,
                enhanced_code: None,
                content: "woot\nmore\ndone".to_string(),
                command: None
            }
        );

        let res = parse_multi_line(&["220-woot", "221-more", "220 done"]).unwrap_err();
        assert!(
            matches!(
                    res,
                ClientError::MalformedResponseLine(ref err) if err == "221-more"
            ),
            "got error {res:?}"
        );

        let res = parse_multi_line(&["220-woot", "220-more", "221 done"]).unwrap_err();
        assert!(
            matches!(
                    res,
                ClientError::MalformedResponseLine(ref err) if err == "221 done"
            ),
            "got error {res:?}"
        );

        assert_eq!(
            parse_multi_line(&["220-4.1.0 woot", "220-more", "220 done",]).unwrap(),
            Response {
                code: 220,
                enhanced_code: Some(EnhancedStatusCode {
                    class: 4,
                    subject: 1,
                    detail: 0
                }),
                content: "woot\nmore\ndone".to_string(),
                command: None
            }
        );

        // Confirm that we strip the enhanced status code from each line
        assert_eq!(
            parse_multi_line(&["220-4.1.0 woot", "220-4.1.0 more", "220 done",]).unwrap(),
            Response {
                code: 220,
                enhanced_code: Some(EnhancedStatusCode {
                    class: 4,
                    subject: 1,
                    detail: 0
                }),
                content: "woot\nmore\ndone".to_string(),
                command: None
            }
        );

        // ... but only if the code matches that of the first line
        assert_eq!(
            parse_multi_line(&["220-4.1.0 woot", "220-4.1.0 more", "220 5.5.5 done",]).unwrap(),
            Response {
                code: 220,
                enhanced_code: Some(EnhancedStatusCode {
                    class: 4,
                    subject: 1,
                    detail: 0
                }),
                content: "woot\nmore\n5.5.5 done".to_string(),
                command: None
            }
        );
    }

    #[test]
    fn test_extract_hostname() {
        assert_eq!(extract_hostname("foo"), "foo");
        assert_eq!(extract_hostname("foo."), "foo");
        assert_eq!(extract_hostname("foo:25"), "foo");
        assert_eq!(extract_hostname("foo.:25"), "foo");
        assert_eq!(extract_hostname("[foo]:25"), "foo");
        assert_eq!(extract_hostname("[foo.]:25"), "foo");
        assert_eq!(extract_hostname("[::1]:25"), "::1");
        assert_eq!(extract_hostname("::1:25"), "::1");
    }

    #[test]
    fn test_format_error_command() {
        assert_eq!(
            format!(
                "{:#}",
                ClientError::TimeOutRequest {
                    commands: vec![Command::DataDot],
                    duration: Duration::from_secs(10),
                }
            ),
            "Timed Out after 10s writing cmd=."
        );
        assert_eq!(
            format!(
                "{:#}",
                ClientError::TimeOutResponse {
                    command: Some(Command::MailFrom {
                        address: {
                            let EnvelopeAddress::Path(p) =
                                EnvelopeAddress::parse("user@host").unwrap()
                            else {
                                panic!("expected Path")
                            };
                            ReversePath::Path(p)
                        },
                        parameters: vec![],
                    }),
                    duration: Duration::from_secs(10),
                }
            ),
            r#"Timed Out waiting 10s for response to cmd=MAIL FROM:<user@host>"#
        );
    }

    #[test]
    fn test_issue_533() {
        let _name = parse_server_name("münchen.de").unwrap();
    }

    /// Adapts an in-memory stream so that it can be used as the socket
    /// of an SmtpClient.
    #[derive(Debug)]
    struct TestStream(tokio::io::DuplexStream);

    impl tokio::io::AsyncRead for TestStream {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl tokio::io::AsyncWrite for TestStream {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    impl AsyncReadAndWrite for TestStream {}

    #[derive(Debug, Default)]
    struct CountingProgress(std::sync::atomic::AtomicUsize);

    impl CountingProgress {
        fn count(&self) -> usize {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl SmtpClientProgress for CountingProgress {
        fn progress(&self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    const TEST_BLOCK_TIMEOUT: Duration = Duration::from_secs(10);

    fn test_payload(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// Returns a client wired to an in-memory peer. The peer end is
    /// returned as a bare DuplexStream.
    fn client_with_peer() -> (SmtpClient, tokio::io::DuplexStream) {
        let (ours, peer) = tokio::io::duplex(DATA_BLOCK_SIZE);
        let timeouts = SmtpClientTimeouts {
            data_block_timeout: TEST_BLOCK_TIMEOUT,
            ..Default::default()
        };
        (
            SmtpClient::with_stream(TestStream(ours), "localhost", timeouts),
            peer,
        )
    }

    /// Spawns a peer that reads up to `chunk` bytes every `interval`.
    /// If `stop_after` is set, it stops reading (but keeps the connection
    /// open) once it has consumed at least that many bytes.
    /// Resolves to everything that was read, once the client closes.
    fn spawn_peer(
        mut peer: tokio::io::DuplexStream,
        chunk: usize,
        interval: Duration,
        stop_after: Option<usize>,
    ) -> tokio::task::JoinHandle<Vec<u8>> {
        tokio::spawn(async move {
            let mut received = vec![];
            let mut buf = vec![0u8; chunk];
            loop {
                if stop_after.map(|n| received.len() >= n).unwrap_or(false) {
                    std::future::pending::<()>().await;
                }
                tokio::time::sleep(interval).await;
                let n = peer.read(&mut buf).await.unwrap();
                if n == 0 {
                    return received;
                }
                received.extend_from_slice(&buf[..n]);
            }
        })
    }

    /// A transfer that takes far longer than data_block_timeout overall,
    /// but in which the peer keeps accepting data, must succeed.
    #[tokio::test(start_paused = true)]
    async fn data_slow_but_steady_peer_succeeds() {
        let (mut client, peer) = client_with_peer();
        let progress = Arc::new(CountingProgress::default());
        client.set_progress_notifier(progress.clone());

        let payload = test_payload(10 * DATA_BLOCK_SIZE);
        // Accept one block's worth of data at a time, at a pace that is
        // slower than data_block_timeout can tolerate for the whole message
        // but comfortably within it for each block.
        let reader = spawn_peer(peer, DATA_BLOCK_SIZE, TEST_BLOCK_TIMEOUT / 2, None);

        let start = tokio::time::Instant::now();
        client.write_data_with_timeout(&payload).await.unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed > TEST_BLOCK_TIMEOUT * 3,
            "expected the transfer to outlast several block timeouts, but took {elapsed:?}"
        );
        assert!(client.is_connected());
        assert_eq!(progress.count(), 10);

        drop(client);
        assert!(
            reader.await.unwrap() == payload,
            "peer received corrupted data"
        );
    }

    /// A peer that stops accepting data must be timed out after
    /// about data_block_timeout, and the connection must not be reused.
    #[tokio::test(start_paused = true)]
    async fn data_stalled_peer_times_out() {
        let (mut client, peer) = client_with_peer();
        let progress = Arc::new(CountingProgress::default());
        client.set_progress_notifier(progress.clone());

        let payload = test_payload(10 * DATA_BLOCK_SIZE);
        // Consume two blocks promptly, then stop reading.
        let _reader = spawn_peer(
            peer,
            DATA_BLOCK_SIZE,
            Duration::from_millis(1),
            Some(2 * DATA_BLOCK_SIZE),
        );

        let start = tokio::time::Instant::now();
        let err = client.write_data_with_timeout(&payload).await.unwrap_err();
        let elapsed = start.elapsed();
        assert!(matches!(err, ClientError::TimeOutData), "got {err:?}");
        assert!(
            elapsed >= TEST_BLOCK_TIMEOUT && elapsed < TEST_BLOCK_TIMEOUT * 2,
            "expected a timeout of about {TEST_BLOCK_TIMEOUT:?}, got {elapsed:?}"
        );
        assert!(!client.is_connected());
        // Blocks that the peer did accept were reported, but no more.
        assert!(progress.count() < 10, "count={}", progress.count());
    }

    /// The payload must arrive intact and in order, whatever its size
    /// relative to the block size, and progress is reported once per block.
    #[tokio::test(start_paused = true)]
    async fn data_block_boundaries() {
        for len in [
            0,
            1,
            DATA_BLOCK_SIZE - 1,
            DATA_BLOCK_SIZE,
            DATA_BLOCK_SIZE + 1,
            3 * DATA_BLOCK_SIZE,
            3 * DATA_BLOCK_SIZE + 17,
        ] {
            let (mut client, peer) = client_with_peer();
            let progress = Arc::new(CountingProgress::default());
            client.set_progress_notifier(progress.clone());
            let reader = spawn_peer(peer, 4096, Duration::from_millis(1), None);

            let payload = test_payload(len);
            client.write_data_with_timeout(&payload).await.unwrap();
            assert_eq!(progress.count(), len.div_ceil(DATA_BLOCK_SIZE), "len={len}");

            drop(client);
            assert!(
                reader.await.unwrap() == payload,
                "peer received corrupted data for len={len}"
            );
        }
    }

    #[test]
    fn data_block_timeout_config() {
        let timeouts: SmtpClientTimeouts = serde_json::from_str("{}").unwrap();
        assert_eq!(timeouts.data_block_timeout, Duration::from_secs(60));
        assert_eq!(timeouts, SmtpClientTimeouts::default());

        let timeouts: SmtpClientTimeouts =
            serde_json::from_str(r#"{"data_block_timeout": "5m"}"#).unwrap();
        assert_eq!(timeouts.data_block_timeout, Duration::from_secs(300));
        // It must not leak into the other timeouts
        assert_eq!(timeouts.data_timeout, Duration::from_secs(300));
        assert_eq!(timeouts.data_dot_timeout, Duration::from_secs(300));

        let timeouts: SmtpClientTimeouts =
            serde_json::from_str(r#"{"data_block_timeout": 90}"#).unwrap();
        assert_eq!(timeouts.data_block_timeout, Duration::from_secs(90));
    }
}
