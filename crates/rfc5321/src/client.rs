use crate::{AsyncReadAndWrite, BoxedAsyncReadAndWrite, Command, Domain, ForwardPath, ReversePath};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls::client::{ServerCertVerified, ServerCertVerifier, WebPkiVerifier};
use tokio_rustls::rustls::{
    Certificate, ClientConfig, OwnedTrustAnchor, RootCertStore, ServerName,
};
use tokio_rustls::TlsConnector;

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
    #[error("invalid DNS name: {0}")]
    InvalidDnsName(#[from] tokio_rustls::rustls::client::InvalidDnsNameError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EsmtpCapability {
    pub name: String,
    pub param: Option<String>,
}

pub struct SmtpClient {
    socket: Option<BoxedAsyncReadAndWrite>,
    hostname: String,
    capabilities: HashMap<String, EsmtpCapability>,
    read_buffer: Vec<u8>,
}

impl SmtpClient {
    pub fn with_stream<S: AsyncReadAndWrite + 'static, H: AsRef<str>>(
        stream: S,
        peer_hostname: H,
    ) -> Self {
        Self {
            socket: Some(Box::new(stream)),
            hostname: peer_hostname.as_ref().to_string(),
            capabilities: HashMap::new(),
            read_buffer: Vec::with_capacity(1024),
        }
    }

    async fn read_line(&mut self) -> Result<String, ClientError> {
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
            let size = self.socket.as_mut().unwrap().read(&mut data).await?;
            if size == 0 {
                self.socket.take();
                return Err(ClientError::NotConnected);
            }
            self.read_buffer.extend_from_slice(&data[0..size]);
        }
    }

    pub async fn read_response(
        &mut self,
        command: Option<String>,
    ) -> Result<Response, ClientError> {
        if let Some(sock) = self.socket.as_mut() {
            sock.flush().await?;
        }

        let mut line = self.read_line().await?;
        let mut parsed = parse_response_line(&line)?;
        let code = parsed.code;
        let (enhanced_code, mut response_string) = match parse_enhanced_status_code(parsed.content)
        {
            Some((enhanced, content)) => (Some(enhanced), content.to_string()),
            None => (None, parsed.content.to_string()),
        };
        while !parsed.is_final {
            line = self.read_line().await?;
            parsed = parse_response_line(&line)?;
            if parsed.code != code {
                return Err(ClientError::MalformedResponseLine(line.to_string()));
            }
            response_string.push('\n');
            response_string.push_str(parsed.content);
        }

        Ok(Response {
            code,
            content: response_string,
            enhanced_code,
            command,
        })
    }

    pub async fn send_command(&mut self, command: &Command) -> Result<Response, ClientError> {
        let line = command.encode();
        match self.socket.as_mut() {
            Some(socket) => {
                socket.write_all(line.as_bytes()).await?;
            }
            None => return Err(ClientError::NotConnected),
        };

        self.read_response(Some(line)).await
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
        let mut encoded_commands: Vec<String> = vec![];

        for cmd in &commands {
            let line = cmd.encode();
            match self.socket.as_mut() {
                Some(socket) => {
                    let res = socket.write_all(line.as_bytes()).await;
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
            if pipeline {
                encoded_commands.push(line);
            } else {
                // Immediately request the response if the server
                // doesn't support pipelining
                results.push(self.read_response(Some(line)).await);
            }
        }

        if pipeline {
            // Now read the responses effectively in a batch
            for line in encoded_commands {
                results.push(self.read_response(Some(line)).await);
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

    pub async fn starttls(&mut self, insecure: bool) -> Result<(), ClientError> {
        let resp = self.send_command(&Command::StartTls).await?;
        if resp.code != 220 {
            return Err(ClientError::Rejected(resp));
        }

        let connector = build_tls_connector(insecure);
        let stream = connector
            .connect(
                ServerName::try_from(self.hostname.as_str())?,
                self.socket.take().unwrap(),
            )
            .await?;
        let stream: BoxedAsyncReadAndWrite = Box::new(stream);
        self.socket.replace(stream);
        Ok(())
    }

    pub async fn send_mail<B: AsRef<[u8]>, SENDER: Into<ReversePath>, RECIP: Into<ForwardPath>>(
        &mut self,
        sender: SENDER,
        recipient: RECIP,
        data: B,
    ) -> Result<Response, ClientError> {
        let mut responses = self
            .pipeline_commands(vec![
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

        let mut needs_stuffing = false;
        let data: &[u8] = data.as_ref();

        for line in data.split(|&b| b == b'\n') {
            if line.starts_with(b".") {
                needs_stuffing = true;
                break;
            }
        }

        if needs_stuffing {
            let mut stuffed = vec![];
            for line in data.split(|&b| b == b'\n') {
                if line.starts_with(b".") {
                    stuffed.push(b'.');
                }
                stuffed.extend_from_slice(line);
            }
            match self.socket.as_mut() {
                Some(sock) => sock.write_all(&stuffed).await?,
                None => return Err(ClientError::NotConnected),
            }
        } else {
            match self.socket.as_mut() {
                Some(sock) => sock.write_all(data).await?,
                None => return Err(ClientError::NotConnected),
            }
        }

        let needs_newline = data.last().map(|&b| b != b'\n').unwrap_or(true);
        let marker = if needs_newline { "\r\n.\r\n" } else { ".\r\n" };

        match self.socket.as_mut() {
            Some(sock) => sock.write_all(marker.as_bytes()).await?,
            None => return Err(ClientError::NotConnected),
        }

        let resp = self.read_response(Some(".".to_string())).await?;
        if resp.code != 250 {
            return Err(ClientError::Rejected(resp));
        }

        Ok(resp)
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone, Copy)]
pub struct EnhancedStatusCode {
    pub class: u8,
    pub subject: u16,
    pub detail: u16,
}

fn parse_enhanced_status_code(line: &str) -> Option<(EnhancedStatusCode, &str)> {
    let mut fields = line.splitn(3, '.');
    let class = fields.next()?.parse::<u8>().ok()?;
    if !matches!(class, 2 | 4 | 5) {
        // No other classes are defined
        return None;
    }
    let subject = fields.next()?.parse::<u16>().ok()?;

    let remainder = fields.next()?;
    let mut fields = remainder.splitn(2, ' ');
    let detail = fields.next()?.parse::<u16>().ok()?;
    let remainder = fields.next()?;

    Some((
        EnhancedStatusCode {
            class,
            subject,
            detail,
        },
        remainder,
    ))
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct Response {
    pub code: u16,
    pub enhanced_code: Option<EnhancedStatusCode>,
    pub content: String,
    pub command: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ResponseLine<'a> {
    pub code: u16,
    pub is_final: bool,
    pub content: &'a str,
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

pub fn build_tls_connector(insecure: bool) -> TlsConnector {
    let config = ClientConfig::builder().with_safe_defaults();

    let verifier: Arc<dyn ServerCertVerifier> = if insecure {
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
        Arc::new(VerifyAll {})
    } else {
        let mut root_cert_store = RootCertStore::empty();

        root_cert_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(
            |ta| {
                OwnedTrustAnchor::from_subject_spki_name_constraints(
                    ta.subject,
                    ta.spki,
                    ta.name_constraints,
                )
            },
        ));
        Arc::new(WebPkiVerifier::new(root_cert_store, None))
    };

    let config = config
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

    TlsConnector::from(Arc::new(config))
}

#[cfg(test)]
mod test {
    use super::*;

    /*
    #[tokio::test]
    async fn test_against_sink() {
        use tokio::net::TcpStream;
        let stream = TcpStream::connect("127.0.0.1:25").await.unwrap();
        let mut client = SmtpClient::with_stream(stream, "localhost");
        dbg!(client.read_response().await).unwrap();
        dbg!(client.ehlo("localhost").await).unwrap();
        let insecure = true;
        dbg!(client.starttls(insecure).await).unwrap();
        let resp = client
            .send_mail(
                ReversePath::try_from("wez@wez").unwrap(),
                ForwardPath::try_from("wez@wez").unwrap(),
                "Subject: hello\r\n\r\nwoot\r\n",
            )
            .await
            .unwrap();
        panic!("{resp:#?}");
    }
    */

    #[test]
    fn response_parsing() {
        assert_eq!(
            parse_enhanced_status_code("2.0.1 w00t"),
            Some((
                EnhancedStatusCode {
                    class: 2,
                    subject: 0,
                    detail: 1
                },
                "w00t"
            ))
        );

        assert_eq!(parse_enhanced_status_code("3.0.0 w00t"), None);

        assert_eq!(parse_enhanced_status_code("2.0.0.1 w00t"), None);

        assert_eq!(parse_enhanced_status_code("2.0.0.1w00t"), None);
    }

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
}
