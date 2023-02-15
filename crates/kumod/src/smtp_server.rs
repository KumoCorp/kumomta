use crate::lua_config::{load_config, LuaConfig};
use crate::queue::QueueManager;
use crate::spool::SpoolManager;
use anyhow::anyhow;
use message::{EnvelopeAddress, Message};
use mlua::ToLuaMulti;
use std::fmt::Debug;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadHalf,
    WriteHalf,
};
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
pub struct SmtpServer<T> {
    reader: BufReader<ReadHalf<T>>,
    writer: BufWriter<WriteHalf<T>>,
    state: Option<TransactionState>,
    said_hello: Option<String>,
    config: LuaConfig,
    hostname: String,
}

#[derive(Debug)]
struct TransactionState {
    sender: EnvelopeAddress,
    recipients: Vec<EnvelopeAddress>,
    meta: serde_json::Value,
}

impl<T: AsyncRead + AsyncWrite + Debug + Send + 'static> SmtpServer<T> {
    pub async fn run(socket: T, hostname: String) -> anyhow::Result<()> {
        let config = load_config().await?;
        let (reader, writer) = tokio::io::split(socket);
        let reader = tokio::io::BufReader::new(reader);
        let writer = tokio::io::BufWriter::new(writer);
        let mut server = SmtpServer {
            reader,
            writer,
            state: None,
            said_hello: None,
            config,
            hostname,
        };

        tokio::task::spawn_local(async move {
            if let Err(err) = server.process().await {
                error!("Error in SmtpServer: {err:#}");
                server
                    .write_response(421, "technical difficulties")
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
        let mut lines = message.as_ref().lines().peekable();
        while let Some(line) = lines.next() {
            let is_last = lines.peek().is_none();
            let sep = if is_last { ' ' } else { '-' };
            let text = format!("{status}{sep}{line}\r\n");
            self.writer.write(text.as_bytes()).await?;
        }
        self.writer.flush().await?;
        Ok(())
    }

    async fn read_line(&mut self) -> anyhow::Result<String> {
        let mut line = String::new();
        self.reader.read_line(&mut line).await?;
        Ok(line)
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
            self.write_response(421, format!("{} Hold on just a moment!", self.hostname))
                .await?;
            return Ok(());
        }

        self.write_response(220, format!("{} KumoMTA\nW00t!\nYeah!", self.hostname))
            .await?;
        loop {
            let line = self.read_line().await?;
            let line = line.trim_end();

            match Command::parse(line) {
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
                Ok(Command::Ehlo(domain)) => {
                    // TODO: we are supposed to report extension commands in our EHLO
                    // response, but we don't have any yet.

                    if let Err(rej) = self
                        .call_callback("smtp_server_ehlo", domain.clone())
                        .await?
                    {
                        self.write_response(rej.code, rej.message).await?;
                        continue;
                    }

                    self.write_response(250, format!("{} Hello {domain}", self.hostname))
                        .await?;
                    self.said_hello.replace(domain);
                }
                Ok(Command::Helo(domain)) => {
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
                Ok(Command::Mail(address)) => {
                    if self.state.is_some() {
                        self.write_response(503, "MAIL FROM already issued; you must RSET first")
                            .await?;
                        continue;
                    }
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
                Ok(Command::Rcpt(address)) => {
                    if self.state.is_none() {
                        self.write_response(503, "MAIL FROM must be issued first")
                            .await?;
                        continue;
                    }
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
                        self.write_response(503, "MAIL FROM must be issued first")
                            .await?;
                        continue;
                    }
                    if self
                        .state
                        .as_ref()
                        .map(|s| s.recipients.is_empty())
                        .unwrap_or(true)
                    {
                        self.write_response(503, "RCPT TO must be issued first")
                            .await?;
                        continue;
                    }
                    self.write_response(354, "Send body; end with CRLF.CRLF")
                        .await?;

                    let mut data = vec![];

                    loop {
                        let line = self.read_line().await?;
                        if line == ".\r\n" {
                            break;
                        }

                        let line = if line.starts_with('.') {
                            &line[1..]
                        } else {
                            &line
                        };

                        data.extend_from_slice(line.as_bytes());
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

                        let queue_name = match message.get_meta("queue")? {
                            serde_json::Value::String(name) => name.to_string(),
                            serde_json::Value::Null => message.recipient()?.domain().to_string(),
                            value => anyhow::bail!(
                                "expected 'queue' metadata to be a string value, got {value:?}"
                            ),
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
                Ok(Command::Noop) => {
                    self.write_response(250, "the goggles do nothing").await?;
                }
                Ok(Command::Unknown(cmd)) => {
                    self.write_response(502, format!("Command unrecognized/unimplemented: {cmd}"))
                        .await?;
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Ehlo(String),
    Helo(String),
    Mail(EnvelopeAddress),
    Rcpt(EnvelopeAddress),
    Data,
    Rset,
    Noop,
    Quit,
    Unknown(String),
}

impl Command {
    fn parse(line: &str) -> anyhow::Result<Self> {
        fn prefix_match(line: &str, candidate: &str) -> bool {
            if line.len() < candidate.len() {
                false
            } else {
                line[..candidate.len()].eq_ignore_ascii_case(candidate)
            }
        }

        fn extract_envelope(line: &str) -> anyhow::Result<(&str, &str)> {
            if !line.starts_with('<') {
                anyhow::bail!("expected <: {line:?}");
            }
            let rangle = line
                .bytes()
                .position(|c| c == b'>')
                .ok_or_else(|| anyhow::anyhow!("expected >: {line:?}"))?;

            Ok((&line[1..rangle], &line[rangle + 1..]))
        }

        Ok(if line.eq_ignore_ascii_case("QUIT") {
            Self::Quit
        } else if line.eq_ignore_ascii_case("DATA") {
            Self::Data
        } else if line.eq_ignore_ascii_case("RSET") {
            Self::Rset
        } else if line.eq_ignore_ascii_case("NOOP") {
            Self::Noop
        } else if prefix_match(line, "EHLO ") {
            Self::Ehlo(line[5..].to_string())
        } else if prefix_match(line, "HELO ") {
            Self::Helo(line[5..].to_string())
        } else if prefix_match(line, "MAIL FROM:") {
            let (address, _params) = extract_envelope(&line[10..])?;
            // TODO: MAIL FROM can accept key/value parameters
            Self::Mail(EnvelopeAddress::parse(address)?)
        } else if prefix_match(line, "RCPT TO:") {
            let (address, _params) = extract_envelope(&line[8..])?;
            if address.is_empty() {
                anyhow::bail!("Null sender not permitted as a recipient");
            }
            Self::Rcpt(EnvelopeAddress::parse(address)?)
        } else {
            Self::Unknown(line.to_string())
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use k9::assert_equal;

    #[test]
    fn command_parser() {
        assert_equal!(Command::parse("QUIT").unwrap(), Command::Quit);
        assert_equal!(Command::parse("quit").unwrap(), Command::Quit);
        assert_equal!(
            Command::parse("quite").unwrap(),
            Command::Unknown("quite".to_string())
        );
        assert_equal!(
            Command::parse("flibble").unwrap(),
            Command::Unknown("flibble".to_string())
        );
        assert_equal!(
            Command::parse("MAIL From:<>").unwrap(),
            Command::Mail(EnvelopeAddress::null_sender())
        );
        assert_equal!(
            Command::parse("MAIL From:<user@example.com>").unwrap(),
            Command::Mail(EnvelopeAddress::parse("user@example.com").unwrap())
        );
        assert_equal!(
            Command::parse("rcpt to:<>").unwrap_err().to_string(),
            "Null sender not permitted as a recipient"
        );
        assert_equal!(
            Command::parse("rcpt TO:<user@example.com>").unwrap(),
            Command::Rcpt(EnvelopeAddress::parse("user@example.com").unwrap())
        );
    }
}
