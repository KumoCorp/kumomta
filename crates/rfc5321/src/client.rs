#![allow(clippy::result_large_err)]
use crate::client_types::*;
use crate::{
    AsyncReadAndWrite, BoxedAsyncReadAndWrite, Command, Domain, EsmtpParameter, ForwardPath,
    ReversePath,
};
use hickory_proto::rr::rdata::tlsa::{CertUsage, Matching, Selector};
use hickory_proto::rr::rdata::TLSA;
use memchr::memmem::Finder;
use openssl::pkey::PKey;
use openssl::ssl::{DaneMatchType, DaneSelector, DaneUsage, SslOptions};
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
use tokio::time::timeout;
use tokio_rustls::rustls::pki_types::ServerName;
use tracing::Level;

pub use crate::tls::TlsOptions;
pub use {openssl, tokio_rustls};

const MAX_LINE_LEN: usize = 4096;

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
    #[error("STARTTLS: {0} is not a valid DNS name")]
    InvalidDnsName(String),
    #[error("Invalid client certificate configured: {error:?}")]
    InvalidClientCertificate { error: String },
    #[error("Timed Out waiting {duration:?} for response to {command:?}")]
    TimeOutResponse {
        command: Option<Command>,
        duration: Duration,
    },
    #[error("Timed Out writing {duration:?} {commands:?}")]
    TimeOutRequest {
        commands: Vec<Command>,
        duration: Duration,
    },
    #[error("Error {error} reading response to {command:?}")]
    ReadError {
        command: Option<Command>,
        error: String,
        partial: String,
    },
    #[error("Error {error} flushing send buffer")]
    FlushError { error: String },
    #[error("Error {error} writing {commands:?}")]
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
    /// Returns the command(s) string suitable for passing into a Response
    pub fn command(&self) -> Option<String> {
        match self {
            Self::TimeOutResponse {
                command: Some(command),
                ..
            }
            | Self::ReadError {
                command: Some(command),
                ..
            } => Some(command.encode()),
            Self::TimeOutRequest { commands, .. } | Self::WriteError { commands, .. }
                if !commands.is_empty() =>
            {
                let commands: Vec<String> = commands.into_iter().map(|cmd| cmd.encode()).collect();
                Some(commands.join(""))
            }
            _ => None,
        }
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
            | Self::InvalidClientCertificate { .. }
            | Self::TimeOutResponse { .. }
            | Self::TimeOutRequest { .. }
            | Self::ReadError { .. }
            | Self::FlushError { .. }
            | Self::WriteError { .. }
            | Self::TimeOutData
            | Self::SslErrorStack(_)
            | Self::NoUsableDaneTlsa { .. } => false,
            Self::Rejected(response) => response.was_due_to_message(),
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
    use_rset: bool,
    enable_rset: bool,
    enable_pipelining: bool,
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
            use_rset: false,
            enable_rset: false,
            enable_pipelining: false,
        }
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
                        self.socket.take();
                        if let Some(tracer) = &self.tracer {
                            tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                                level: Level::ERROR,
                                message: format!("Read Timeout after {timeout_duration:?}"),
                            });
                            tracer.trace_event(SmtpClientTraceEvent::Closed);
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

    pub async fn read_response(
        &mut self,
        command: Option<&Command>,
        timeout_duration: Duration,
    ) -> Result<Response, ClientError> {
        if let Some(sock) = self.socket.as_mut() {
            if let Err(err) = sock.flush().await {
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
        }

        let mut line = self.read_line(timeout_duration, command).await?;
        tracing::trace!("recv<-{}: {line}", self.hostname);
        let mut parsed = parse_response_line(&line)?;
        let mut response_builder = ResponseBuilder::new(&parsed);

        let subsequent_line_timeout_duration = Duration::from_secs(60).min(timeout_duration);
        while !parsed.is_final {
            line = self
                .read_line(subsequent_line_timeout_duration, command)
                .await?;
            parsed = parse_response_line(&line)?;
            response_builder
                .add_line(&parsed)
                .map_err(ClientError::MalformedResponseLine)?;
        }

        let response = response_builder.build(command.map(|cmd| cmd.encode()));

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
        let mut all = String::new();
        for cmd in commands {
            let line = cmd.encode();
            all.push_str(&line);
            lines.push(line);
        }
        tracing::trace!("send->{}: (PIPELINE) {all}", self.hostname);
        if self.socket.is_some() {
            if let Some(tracer) = &self.tracer {
                // Send the lines individually to the tracer, so that we
                // don't break --terse mode
                for line in lines {
                    WriteTracer::trace(tracer, &line);
                }
            }
        }
        self.write_all_with_timeout(
            total_timeout,
            all.as_bytes(),
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
                WriteTracer::trace(tracer, &line);
            }
        }

        let timeout_duration = command.client_timeout_request(&self.timeouts);
        self.write_all_with_timeout(
            timeout_duration,
            line.as_bytes(),
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
        let timeout_duration = Command::Data.client_timeout_request(&self.timeouts);
        self.write_all_with_timeout(
            timeout_duration,
            data,
            || ClientError::TimeOutData,
            |error| ClientError::WriteError {
                error,
                commands: vec![],
            },
        )
        .await
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
            .send_command(&Command::Lhlo(Domain::Name(ehlo_name.to_string())))
            .await?;
        self.ehlo_common(response)
    }

    pub async fn ehlo(
        &mut self,
        ehlo_name: &str,
    ) -> Result<&HashMap<String, EsmtpCapability>, ClientError> {
        let response = self
            .send_command(&Command::Ehlo(Domain::Name(ehlo_name.to_string())))
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

    /// Attempt TLS handshake.
    /// Returns Err for IO errors.
    /// On completion, return an option that will be:
    /// * Some(handshake_error) - if the handshake failed
    /// * None - if the handshake succeeded
    pub async fn starttls(&mut self, options: TlsOptions) -> Result<TlsStatus, ClientError> {
        let resp = self.send_command(&Command::StartTls).await?;
        if resp.code != 220 {
            return Err(ClientError::Rejected(resp));
        }

        let mut handshake_error = None;
        let mut tls_info = TlsInformation::default();

        let stream: BoxedAsyncReadAndWrite = if options.prefer_openssl
            || !options.dane_tlsa.is_empty()
        {
            let connector = options
                .build_openssl_connector(&self.hostname)
                .map_err(|error| ClientError::InvalidClientCertificate {
                    error: error.to_string(),
                })?;
            let ssl = connector.into_ssl(self.hostname.as_str())?;

            let (stream, dup_stream) = match self.socket.take() {
                Some(s) => {
                    let d = s.try_dup();
                    (s, d)
                }
                None => return Err(ClientError::NotConnected),
            };

            let mut ssl_stream = tokio_openssl::SslStream::new(ssl, stream)?;

            if let Err(err) = std::pin::Pin::new(&mut ssl_stream).connect().await {
                handshake_error.replace(format!("{err:#}"));
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

            match (&handshake_error, dup_stream) {
                (Some(_), Some(dup_stream)) if !ssl_stream.ssl().is_init_finished() => {
                    // Try falling back to clear text on the duplicate stream.
                    // This is imperfect: in a failed validation scenario we will
                    // end up trying to read binary data as a string and get a UTF-8
                    // error if the peer thinks the session is encrypted.
                    drop(ssl_stream);
                    Box::new(dup_stream)
                }
                _ => Box::new(ssl_stream),
            }
        } else {
            tls_info.provider_name = "rustls".to_string();
            let connector = options.build_tls_connector().await.map_err(|error| {
                ClientError::InvalidClientCertificate {
                    error: error.to_string(),
                }
            })?;
            let server_name = match IpAddr::from_str(self.hostname.as_str()) {
                Ok(ip) => ServerName::IpAddress(ip.into()),
                Err(_) => ServerName::try_from(self.hostname.clone())
                    .map_err(|_| ClientError::InvalidDnsName(self.hostname.clone()))?,
            };

            match connector
                .connect(
                    server_name,
                    match self.socket.take() {
                        Some(s) => s,
                        None => return Err(ClientError::NotConnected),
                    },
                )
                .into_fallible()
                .await
            {
                Ok(stream) => {
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

                    Box::new(stream)
                }
                Err((err, stream)) => {
                    handshake_error.replace(format!("{err:#}"));
                    stream
                }
            }
        };

        if let Some(tracer) = &self.tracer {
            tracer.trace_event(SmtpClientTraceEvent::Diagnostic {
                level: Level::INFO,
                message: match &handshake_error {
                    Some(error) => format!("STARTTLS handshake failed: {error:?}"),
                    None => format!("STARTTLS handshake -> {tls_info:?}"),
                },
            });
        }

        self.socket.replace(stream);
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
        let sender = sender.into();
        let recipient = recipient.into();

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
        let envelope_is_8bit = !sender.is_ascii() || !recipient.is_ascii();

        let mut mail_from_params = vec![];
        if data_is_8bit && self.capabilities.contains_key("8BITMIME") {
            mail_from_params.push(EsmtpParameter {
                name: "BODY".to_string(),
                value: Some("8BITMIME".to_string()),
            });
        }

        if envelope_is_8bit && self.capabilities.contains_key("SMTPUTF8") {
            mail_from_params.push(EsmtpParameter {
                name: "SMTPUTF8".to_string(),
                value: None,
            });
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
        commands.push(Command::RcptTo {
            address: recipient,
            parameters: vec![],
        });
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

        let rcpt_resp = responses.remove(0)?;
        if is_err && rcpt_resp.code != 250 {
            return Err(ClientError::Rejected(rcpt_resp));
        }

        let data_resp = responses.remove(0)?;
        if is_err && data_resp.code != 354 {
            return Err(ClientError::Rejected(data_resp));
        }

        if data_resp.code == 354 && (mail_resp.code != 250 || rcpt_resp.code != 250) {
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
        if rcpt_resp.code != 250 {
            return Err(ClientError::Rejected(rcpt_resp));
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

        Ok(resp)
    }
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
fn parse_response_line(line: &str) -> Result<ResponseLine, ClientError> {
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

impl TlsOptions {
    pub fn build_openssl_connector(
        &self,
        hostname: &str,
    ) -> Result<openssl::ssl::ConnectConfiguration, ClientError> {
        tracing::trace!("build_openssl_connector for {hostname}");
        let mut builder =
            openssl::ssl::SslConnector::builder(openssl::ssl::SslMethod::tls_client())?;

        if let (Some(cert_data), Some(key_data)) =
            (&self.certificate_from_pem, &self.private_key_from_pem)
        {
            let cert = X509::from_pem(cert_data)?;
            builder.set_certificate(&cert)?;

            let key = PKey::private_key_from_pem(key_data)?;
            builder.set_private_key(&key)?;

            builder.check_private_key()?;
        }

        if let Some(list) = &self.openssl_cipher_list {
            builder.set_cipher_list(list)?;
        }

        if let Some(suites) = &self.openssl_cipher_suites {
            builder.set_ciphersuites(suites)?;
        }

        if let Some(options) = &self.openssl_options {
            builder.clear_options(SslOptions::all());
            builder.set_options(*options);
        }

        if self.insecure {
            builder.set_verify(openssl::ssl::SslVerifyMode::NONE);
        }

        if !self.dane_tlsa.is_empty() {
            builder.dane_enable()?;
            builder.set_no_dane_ee_namechecks();
        }

        let connector = builder.build();

        let mut config = connector.configure()?;

        if !self.dane_tlsa.is_empty() {
            config.dane_enable(hostname)?;
            let mut any_usable = false;
            for tlsa in &self.dane_tlsa {
                let usable = config.dane_tlsa_add(
                    match tlsa.cert_usage() {
                        CertUsage::PkixTa => DaneUsage::PKIX_TA,
                        CertUsage::PkixEe => DaneUsage::PKIX_EE,
                        CertUsage::DaneTa => DaneUsage::DANE_TA,
                        CertUsage::DaneEe => DaneUsage::DANE_EE,
                        CertUsage::Unassigned(n) => DaneUsage::from_raw(n),
                        CertUsage::Private => DaneUsage::PRIV_CERT,
                    },
                    match tlsa.selector() {
                        Selector::Full => DaneSelector::CERT,
                        Selector::Spki => DaneSelector::SPKI,
                        Selector::Unassigned(n) => DaneSelector::from_raw(n),
                        Selector::Private => DaneSelector::PRIV_SEL,
                    },
                    match tlsa.matching() {
                        Matching::Raw => DaneMatchType::FULL,
                        Matching::Sha256 => DaneMatchType::SHA2_256,
                        Matching::Sha512 => DaneMatchType::SHA2_512,
                        Matching::Unassigned(n) => DaneMatchType::from_raw(n),
                        Matching::Private => DaneMatchType::PRIV_MATCH,
                    },
                    tlsa.cert_data(),
                )?;

                tracing::trace!("build_dane_connector usable={usable} {tlsa:?}");
                if usable {
                    any_usable = true;
                }
            }

            if !any_usable {
                return Err(ClientError::NoUsableDaneTlsa {
                    hostname: hostname.to_string(),
                    tlsa: self.dane_tlsa.clone(),
                });
            }
        }

        Ok(config)
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
/// ```norun
/// ["C=US", "ST=CA", "L=SanFrancisco", "O=Fort-Funston", "OU=MyOrganizationalUnit",
/// "CN=do.havedane.net", "name=EasyRSA", "emailAddress=me@myhost.mydomain"]
/// ```
fn subject_name(cert: &X509Ref) -> Vec<String> {
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

#[cfg(test)]
mod test {
    use super::*;

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
}
