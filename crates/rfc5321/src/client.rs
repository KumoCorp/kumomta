use crate::client_types::*;
use crate::{AsyncReadAndWrite, BoxedAsyncReadAndWrite, Command, Domain, ForwardPath, ReversePath};
use hickory_proto::rr::rdata::tlsa::{CertUsage, Matching, Selector};
use hickory_proto::rr::rdata::TLSA;
use memchr::memmem::Finder;
use once_cell::sync::Lazy;
use openssl::ssl::{DaneMatchType, DaneSelector, DaneUsage};
use openssl::x509::{X509Ref, X509};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::time::timeout;
use tokio_rustls::rustls::client::{ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::{
    Certificate, ClientConfig, OwnedTrustAnchor, RootCertStore, ServerName,
};
use tokio_rustls::TlsConnector;
use tracing::Level;

const MAX_LINE_LEN: usize = 4096;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("response is not UTF8")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
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
    #[error("Timed Out waiting {duration:?} for response to {command:?}")]
    TimeOutResponse {
        command: Option<Command>,
        duration: Duration,
    },
    #[error("Timed Out writing {duration:?} {command:?}")]
    TimeOutRequest {
        command: Command,
        duration: Duration,
    },
    #[error("Timed Out sending message payload data")]
    TimeOutData,
    #[error("SSL Error: {0}")]
    SslErrorStack(#[from] openssl::error::ErrorStack),
    #[error("SSL Error: {0}")]
    SslError(#[from] openssl::ssl::Error),
    #[error("No usable DANE TLSA records for {hostname}: {tlsa:?}")]
    NoUsableDaneTlsa { hostname: String, tlsa: Vec<TLSA> },
}

#[derive(Debug, Clone, Default)]
pub struct TlsOptions {
    pub insecure: bool,
    pub alt_name: Option<String>,
    pub dane_tlsa: Vec<TLSA>,
    pub prefer_openssl: bool,
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
impl<'a> DeferredTracer for WriteTracer<'a> {
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
impl<'a> DeferredTracer for BinWriteTracer<'a> {
    fn trace(&self) -> SmtpClientTraceEvent {
        let data = String::from_utf8_lossy(&self.data).to_string();
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
impl<'a> DeferredTracer for ReadTracer<'a> {
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
}

fn extract_hostname(hostname: &str) -> &str {
    // Just the hostname, without any :port
    let fields: Vec<&str> = hostname.rsplitn(2, ':').collect();
    let hostname = if fields.len() == 2 {
        fields[1]
    } else {
        hostname
    };

    if hostname.starts_with('[') && hostname.ends_with(']') {
        &hostname[1..hostname.len() - 1]
    } else {
        hostname
    }
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
        }
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
                    Ok(result) => result?,
                    Err(_) => {
                        return Err(ClientError::TimeOutResponse {
                            command: cmd.cloned(),
                            duration: timeout_duration,
                        })
                    }
                },
                None => return Err(ClientError::NotConnected),
            };
            if size == 0 {
                self.socket.take();
                if let Some(tracer) = &self.tracer {
                    tracer.trace_event(SmtpClientTraceEvent::Closed);
                }
                return Err(ClientError::NotConnected);
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
            sock.flush().await?;
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
        let line = command.encode();
        tracing::trace!("send->{}: {line}", self.hostname);
        match self.socket.as_mut() {
            Some(socket) => {
                if let Some(tracer) = &self.tracer {
                    WriteTracer::trace(tracer, &line);
                }

                match timeout(
                    command.client_timeout_request(&self.timeouts),
                    socket.write_all(line.as_bytes()),
                )
                .await
                {
                    Ok(result) => result.map_err(|_| ClientError::NotConnected)?,
                    Err(_) => {
                        return Err(ClientError::TimeOutRequest {
                            command: command.clone(),
                            duration: command.client_timeout_request(&self.timeouts),
                        })
                    }
                }
            }
            None => return Err(ClientError::NotConnected),
        };

        self.read_response(Some(command), command.client_timeout(&self.timeouts))
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
        let pipeline = self.capabilities.contains_key("PIPELINING");
        let mut results: Vec<Result<Response, ClientError>> = vec![];

        for cmd in &commands {
            let line = cmd.encode();
            tracing::trace!(
                "send->{}: {}{line}",
                self.hostname,
                if pipeline { "(PIPELINE) " } else { "" },
            );
            match self.socket.as_mut() {
                Some(socket) => {
                    if let Some(tracer) = &self.tracer {
                        WriteTracer::trace(tracer, &line);
                    }

                    let res = match timeout(
                        cmd.client_timeout_request(&self.timeouts),
                        socket.write_all(line.as_bytes()),
                    )
                    .await
                    {
                        Ok(result) => result.map_err(|_| ClientError::NotConnected),
                        Err(_) => Err(ClientError::TimeOutRequest {
                            command: cmd.clone(),
                            duration: cmd.client_timeout_request(&self.timeouts),
                        }),
                    };
                    if let Err(err) = res {
                        results.push(Err(err.into()));
                        return results;
                    }
                }
                None => {
                    results.push(Err(ClientError::NotConnected));
                    return results;
                }
            };
            if !pipeline {
                // Immediately request the response if the server
                // doesn't support pipelining
                results.push(
                    self.read_response(Some(cmd), cmd.client_timeout(&self.timeouts))
                        .await,
                );
            }
        }

        if pipeline {
            // Now read the responses effectively in a batch
            for cmd in &commands {
                results.push(
                    self.read_response(Some(cmd), cmd.client_timeout(&self.timeouts))
                        .await,
                );
            }
        }

        results
    }

    pub async fn ehlo(
        &mut self,
        ehlo_name: &str,
    ) -> Result<&HashMap<String, EsmtpCapability>, ClientError> {
        let response = self
            .send_command(&Command::Ehlo(Domain::Name(ehlo_name.to_string())))
            .await?;
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

        let stream: BoxedAsyncReadAndWrite =
            if options.prefer_openssl || !options.dane_tlsa.is_empty() {
                let connector = build_openssl_connector(&options, &self.hostname)?;
                let ssl = connector.into_ssl(self.hostname.as_str())?;
                let mut ssl_stream = tokio_openssl::SslStream::new(
                    ssl,
                    match self.socket.take() {
                        Some(s) => s,
                        None => return Err(ClientError::NotConnected),
                    },
                )?;

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

                Box::new(ssl_stream)
            } else {
                tls_info.provider_name = "rustls".to_string();
                let connector = build_tls_connector(&options);
                let server_name = match IpAddr::from_str(self.hostname.as_str()) {
                    Ok(ip) => ServerName::IpAddress(ip),
                    Err(_) => ServerName::try_from(self.hostname.as_str())
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
        let mut responses = self
            .pipeline_commands(vec![
                Command::Rset,
                Command::MailFrom {
                    address: sender.into(),
                    parameters: vec![],
                },
                Command::RcptTo {
                    address: recipient.into(),
                    parameters: vec![],
                },
                Command::Data,
            ])
            .await;

        if responses.is_empty() {
            // Should be impossible to get here really, but if we do,
            // assume that we aren't connected
            return Err(ClientError::NotConnected);
        }

        let rset_resp = responses.remove(0)?;
        if rset_resp.code != 250 {
            return Err(ClientError::Rejected(rset_resp));
        }

        let mail_resp = responses.remove(0)?;
        if mail_resp.code != 250 {
            return Err(ClientError::Rejected(mail_resp));
        }

        let rcpt_resp = responses.remove(0)?;
        if rcpt_resp.code != 250 {
            return Err(ClientError::Rejected(rcpt_resp));
        }

        let data_resp = responses.remove(0)?;
        if data_resp.code != 354 {
            return Err(ClientError::Rejected(data_resp));
        }

        let data: &[u8] = data.as_ref();
        let stuffed;

        let data = match apply_dot_stuffing(data) {
            Some(d) => {
                stuffed = d;
                &stuffed
            }
            None => data,
        };
        let needs_newline = data.last().map(|&b| b != b'\n').unwrap_or(true);

        tracing::trace!("message data is {} bytes", data.len());

        match self.socket.as_mut() {
            Some(sock) => {
                if let Some(tracer) = &self.tracer {
                    BinWriteTracer::trace(tracer, &data);
                }

                match timeout(
                    Command::Data.client_timeout_request(&self.timeouts),
                    sock.write_all(data),
                )
                .await
                {
                    Ok(result) => result.map_err(|_| ClientError::NotConnected)?,
                    Err(_) => return Err(ClientError::TimeOutData),
                }
            }
            None => return Err(ClientError::NotConnected),
        }

        let marker = if needs_newline { "\r\n.\r\n" } else { ".\r\n" };

        tracing::trace!("send->{}: {}", self.hostname, marker.escape_debug());

        match self.socket.as_mut() {
            Some(sock) => {
                if let Some(tracer) = &self.tracer {
                    WriteTracer::trace(tracer, &marker);
                }

                match timeout(
                    Command::Data.client_timeout_request(&self.timeouts),
                    sock.write_all(marker.as_bytes()),
                )
                .await
                {
                    Ok(result) => result.map_err(|_| ClientError::NotConnected)?,
                    Err(_) => {
                        return Err(ClientError::TimeOutRequest {
                            command: Command::Data,
                            duration: Command::Data.client_timeout_request(&self.timeouts),
                        })
                    }
                }
            }
            None => return Err(ClientError::NotConnected),
        }

        let data_dot = Command::DataDot;
        let resp = self
            .read_response(Some(&data_dot), data_dot.client_timeout(&self.timeouts))
            .await?;
        if resp.code != 250 {
            return Err(ClientError::Rejected(resp));
        }

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

pub fn build_openssl_connector(
    options: &TlsOptions,
    hostname: &str,
) -> Result<openssl::ssl::ConnectConfiguration, ClientError> {
    tracing::trace!("build_openssl_connector for {hostname}");
    let mut builder = openssl::ssl::SslConnector::builder(openssl::ssl::SslMethod::tls_client())?;

    if options.insecure {
        builder.set_verify(openssl::ssl::SslVerifyMode::NONE);
    }

    if !options.dane_tlsa.is_empty() {
        builder.dane_enable()?;
        builder.set_no_dane_ee_namechecks();
    }

    let connector = builder.build();

    let mut config = connector.configure()?;

    if !options.dane_tlsa.is_empty() {
        config.dane_enable(hostname)?;
        let mut any_usable = false;
        for tlsa in &options.dane_tlsa {
            let usable = config.dane_tlsa_add(
                match tlsa.cert_usage() {
                    CertUsage::CA => DaneUsage::PKIX_TA,
                    CertUsage::Service => DaneUsage::PKIX_EE,
                    CertUsage::TrustAnchor => DaneUsage::DANE_TA,
                    CertUsage::DomainIssued => DaneUsage::DANE_EE,
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
                tlsa: options.dane_tlsa.clone(),
            });
        }
    }

    Ok(config)
}

pub fn build_tls_connector(options: &TlsOptions) -> TlsConnector {
    let mut root_store = RootCertStore::empty();
    root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| {
        OwnedTrustAnchor::from_subject_spki_name_constraints(
            ta.subject,
            ta.spki,
            ta.name_constraints,
        )
    }));

    let mut config = ClientConfig::builder()
        .with_cipher_suites(tokio_rustls::rustls::DEFAULT_CIPHER_SUITES)
        .with_safe_default_kx_groups()
        .with_protocol_versions(tokio_rustls::rustls::DEFAULT_VERSIONS)
        .expect("inconsistent cipher-suite/versions selected")
        .with_root_certificates(root_store)
        .with_no_client_auth();

    if options.insecure {
        struct VerifyAll;
        impl ServerCertVerifier for VerifyAll {
            fn verify_server_cert(
                &self,
                _: &Certificate,
                _: &[Certificate],
                _: &ServerName,
                _: &mut dyn Iterator<Item = &[u8]>,
                _: &[u8],
                _: std::time::SystemTime,
            ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
                Ok(ServerCertVerified::assertion())
            }
        }
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(VerifyAll {}));
    }

    TlsConnector::from(Arc::new(config))
}

fn apply_dot_stuffing(data: &[u8]) -> Option<Vec<u8>> {
    static LFDOT: Lazy<Finder> = Lazy::new(|| memchr::memmem::Finder::new("\n."));

    if !data.starts_with(b".") && LFDOT.find(&data).is_none() {
        return None;
    }

    let mut stuffed = vec![];
    if data.starts_with(b".") {
        stuffed.push(b'.');
    }
    let mut last_idx = 0;
    for i in LFDOT.find_iter(&data) {
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
        assert_eq!(extract_hostname("foo:25"), "foo");
        assert_eq!(extract_hostname("[foo]:25"), "foo");
        assert_eq!(extract_hostname("[::1]:25"), "::1");
        assert_eq!(extract_hostname("::1:25"), "::1");
    }
}
