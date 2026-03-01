#![cfg(test)]
use crate::tsa::{TsaArgs, TsaDaemon};
use crate::webhook::WebHookServer;
use anyhow::Context;
use futures::stream::FusedStream;
use futures::{SinkExt, StreamExt};
use kumo_api_client::KumoApiClient;
use kumo_api_types::{TraceSmtpV1Event, TraceSmtpV1Request};
use kumo_log_types::*;
use kumo_server_common::acct::AcctLogRecord;
use maildir::{MailEntry, Maildir};
use mailparsing::MessageBuilder;
use nix::unistd::{Uid, User};
use parking_lot::Mutex;
use rfc5321::{
    BatchSendSuccess, ForwardPath, Response, ReversePath, SmtpClient, SmtpClientTimeouts,
};
use sqlite::{Connection, State};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Default, Clone)]
pub struct MailGenParams<'a> {
    pub sender: Option<&'a str>,
    pub recip: Option<&'a str>,
    pub size: Option<usize>,
    pub wrap: Option<usize>,
    pub subject: Option<&'a str>,
    pub body: Option<&'a str>,
    pub full_content: Option<&'a str>,
    pub ignore_8bit_checks: bool,
    pub recip_list: Option<Vec<&'a str>>,
}

/// Generate a single nonsense string with no spaces with
/// length at least that specified
#[allow(dead_code)]
pub fn generate_nonsense_string(n_bytes: usize) -> String {
    let mut chain = lipsum::MarkovChain::new();
    chain.learn(lipsum::LIBER_PRIMUS);
    let mut result = String::new();
    for word in chain.iter() {
        result.push_str(word);
        if result.len() >= n_bytes {
            return result;
        }
    }
    unreachable!();
}

/// Generate text suitable for use as a message body of at
/// least n_bytes, wrapped to wrap columns using CRLF as
/// the line endings
pub fn generate_message_text(n_bytes: usize, wrap: usize) -> String {
    let mut chain = lipsum::MarkovChain::new();
    chain.learn(lipsum::LIBER_PRIMUS);
    let mut result = String::new();
    for word in chain.iter() {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(word);
        if result.len() >= n_bytes {
            let mut result = textwrap::fill(
                &result,
                textwrap::Options::new(wrap).line_ending(textwrap::LineEnding::CRLF),
            );
            result.push_str("\r\n");
            return result;
        }
    }
    unreachable!();
}

impl MailGenParams<'_> {
    pub fn recipient_list(&self) -> Vec<&str> {
        let mut recipients = vec![];
        if let Some(recip) = &self.recip {
            recipients.push(*recip);
        }
        if let Some(recips) = &self.recip_list {
            for &r in recips {
                recipients.push(r);
            }
        }
        if recipients.is_empty() {
            recipients.push("recip@example.com");
        }
        recipients
    }

    pub async fn send(&self, client: &mut SmtpClient) -> anyhow::Result<Response> {
        client.set_ignore_8bit_checks(self.ignore_8bit_checks);
        let sender = self.sender.unwrap_or("sender@example.com");
        let recips = self.recipient_list();
        anyhow::ensure!(recips.len() == 1, "use send_batch for multi-recipient!");
        let recip = recips[0];
        let body = self
            .generate()
            .context("generation of message body failed")?;

        Ok(client
            .send_mail(
                ReversePath::try_from(sender).unwrap(),
                ForwardPath::try_from(recip).unwrap(),
                &body,
            )
            .await?)
    }

    pub async fn send_batch(&self, client: &mut SmtpClient) -> anyhow::Result<BatchSendSuccess> {
        let sender = self.sender.unwrap_or("sender@example.com");
        let recips = self
            .recipient_list()
            .into_iter()
            .map(|r| ForwardPath::try_from(r).unwrap())
            .collect();
        let body = self
            .generate()
            .context("generation of message body failed")?;

        Ok(client
            .send_mail_multi_recip(ReversePath::try_from(sender).unwrap(), recips, &body)
            .await?)
    }

    pub fn generate(&self) -> anyhow::Result<String> {
        if let Some(full) = &self.full_content {
            return Ok(String::from_utf8(mailparsing::normalize_crlf(
                full.as_bytes(),
            ))?);
        }
        let body_owner;
        let body = if let Some(b) = self.body {
            b
        } else {
            let size = self.size.unwrap_or(1024);
            let wrap = self.wrap.unwrap_or(78);
            body_owner = generate_message_text(size, wrap);
            &body_owner
        };
        let sender = self.sender.unwrap_or("sender@example.com");
        let recip = self.recipient_list().join(", ");
        let mut message = MessageBuilder::new();
        message.set_from(sender)?;
        message.set_to(&*recip).ok(); // the no_ports test assigns an address that is invalid in To:
        message.set_subject(self.subject.unwrap_or("Hello! This is a test"))?;
        message.text_plain(body);
        message.prepend("X-Test1", "Test1");
        message.prepend("X-Another", "Another");
        message.set_stable_content(true);
        Ok(message.build()?.to_message_string())
    }
}

#[derive(Debug, PartialEq)]
#[allow(unused)]
pub struct DeliverySummary {
    pub source_counts: BTreeMap<RecordType, usize>,
    pub sink_counts: BTreeMap<RecordType, usize>,
}

pub struct DaemonWithMaildir {
    pub source: KumoDaemon,
    pub sink: KumoDaemon,
}

pub fn target_bin(tool: &str) -> anyhow::Result<PathBuf> {
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or("../../target".to_string());
    let path = if cfg!(debug_assertions) {
        format!("{target}/debug/{tool}")
    } else {
        format!("{target}/release/{tool}")
    };
    std::fs::canonicalize(&path).with_context(|| format!("canonicalize {path}"))
}

pub struct DaemonWithMaildirOptions {
    policy_file: String,
    env: Vec<(String, String)>,
}

impl DaemonWithMaildirOptions {
    pub fn new() -> Self {
        Self {
            policy_file: "source.lua".to_string(),
            env: vec![],
        }
    }

    #[allow(unused)]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn policy_file(mut self, file: impl Into<String>) -> Self {
        self.policy_file = file.into();
        self
    }

    pub async fn start(self) -> anyhow::Result<DaemonWithMaildir> {
        DaemonWithMaildir::start_with_options(self).await
    }
}

impl DaemonWithMaildir {
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with_env(vec![]).await
    }

    pub async fn start_with_options(options: DaemonWithMaildirOptions) -> anyhow::Result<Self> {
        let mut env = options.env;

        let sink = KumoDaemon::spawn_maildir_env(env.clone())
            .await
            .context("spawn_maildir")?;
        let smtp = sink.listener("smtp");
        env.push(("KUMOD_SMTP_SINK_PORT".to_string(), smtp.port().to_string()));
        let http = sink.listener("http");
        env.push(("KUMOD_HTTP_SINK_PORT".to_string(), http.port().to_string()));

        let source = KumoDaemon::spawn(KumoArgs {
            policy_file: options.policy_file,
            env,
        })
        .await
        .context("KumoDaemon::spawn")?;

        Ok(Self { source, sink })
    }

    pub async fn start_with_env(env: Vec<(&str, &str)>) -> anyhow::Result<Self> {
        DaemonWithMaildirOptions {
            env: env
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..DaemonWithMaildirOptions::new()
        }
        .start()
        .await
    }

    pub async fn smtp_client(&self) -> anyhow::Result<SmtpClient> {
        self.source.smtp_client("localhost").await
    }

    pub async fn kcli(
        &self,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> anyhow::Result<std::process::ExitStatus> {
        let path = target_bin("kcli")?;
        let mut cmd = Command::new(path);
        cmd.args([
            "--endpoint",
            &format!("http://{}", self.source.listener("http")),
        ]);
        cmd.args(args);
        let label = format!("{cmd:?}");
        let status = cmd.status().await?;
        anyhow::ensure!(status.success(), "{label}: {status:?}");
        Ok(status)
    }

    pub async fn trace_server(&self) -> anyhow::Result<ServerTracer> {
        return Ok(self.source.trace_server().await?);
    }

    pub async fn trace_sink(&self) -> anyhow::Result<ServerTracer> {
        return Ok(self.sink.trace_server().await?);
    }

    pub async fn kcli_json<R: for<'a> serde::Deserialize<'a>>(
        &self,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> anyhow::Result<R> {
        self.source.kcli_json(args).await
    }

    pub async fn sink_kcli_json<R: for<'a> serde::Deserialize<'a>>(
        &self,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> anyhow::Result<R> {
        self.sink.kcli_json(args).await
    }

    pub fn extract_maildir_messages(&self) -> anyhow::Result<Vec<MailEntry>> {
        let mut messages = vec![];
        let md = self.sink.maildir();
        for entry in md.list_new() {
            messages.push(entry?);
        }
        Ok(messages)
    }

    pub async fn wait_for_maildir_count(&self, count: usize, timeout: Duration) -> bool {
        self.sink.wait_for_maildir_count(count, timeout).await
    }

    pub async fn wait_for_source_summary<F>(&self, mut func: F, timeout: Duration) -> bool
    where
        F: FnMut(&BTreeMap<RecordType, usize>) -> bool,
    {
        tokio::select! {
            _ = async {
                loop {
                    if let Ok(summary) = self.source.summarize_logs().await {
                        let done = (func)(&summary);
                        if done {
                            return true;
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }

    pub async fn wait_for_sink_summary<F>(&self, mut func: F, timeout: Duration) -> bool
    where
        F: FnMut(&BTreeMap<RecordType, usize>) -> bool,
    {
        tokio::select! {
            _ = async {
                loop {
                    if let Ok(summary) = self.sink.summarize_logs().await {
                        let done = (func)(&summary);
                        if done {
                            return true;
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }

    pub async fn stop_both(&mut self) -> anyhow::Result<()> {
        let (res_1, res_2) = tokio::join!(self.source.stop(), self.sink.stop());
        res_1?;
        res_2?;
        eprintln!("stopped_both");
        Ok(())
    }

    pub async fn dump_logs(&self) -> anyhow::Result<DeliverySummary> {
        eprintln!("source logs:");
        let source_counts = self.source.dump_logs().await?;
        eprintln!("sink logs:");
        let sink_counts = self.sink.dump_logs().await?;
        Ok(DeliverySummary {
            source_counts,
            sink_counts,
        })
    }

    /// Raise an error if the acct log has any denied access entries.
    /// That implies that there is an issue with the ACL definitions
    pub async fn assert_no_acct_deny(&self) -> anyhow::Result<()> {
        self.source.assert_no_acct_deny().await?;
        self.sink.assert_no_acct_deny().await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct KumoDaemon {
    pub dir: TempDir,
    pub listeners: BTreeMap<String, SocketAddr>,
    child: Child,
}

#[derive(Default, Debug)]
pub struct KumoArgs {
    pub policy_file: String,
    pub env: Vec<(String, String)>,
}

impl KumoDaemon {
    pub async fn spawn_maildir() -> anyhow::Result<Self> {
        KumoDaemon::spawn_maildir_env(vec![]).await
    }

    pub async fn spawn_maildir_env(env: Vec<(String, String)>) -> anyhow::Result<Self> {
        KumoDaemon::spawn(KumoArgs {
            policy_file: "maildir-sink.lua".to_string(),
            env,
        })
        .await
    }

    pub async fn spawn_with_policy(policy_file: impl AsRef<Path>) -> anyhow::Result<Self> {
        KumoDaemon::spawn(KumoArgs {
            policy_file: policy_file.as_ref().to_string_lossy().to_string(),
            env: vec![],
        })
        .await
    }

    #[allow(dead_code)]
    pub async fn spawn_sink() -> anyhow::Result<Self> {
        KumoDaemon::spawn(KumoArgs {
            policy_file: "sink.lua".to_string(),
            env: vec![],
        })
        .await
    }

    pub async fn spawn(args: KumoArgs) -> anyhow::Result<Self> {
        let path = target_bin("kumod")?;

        let dir = tempfile::tempdir().context("make temp dir")?;

        let user = User::from_uid(Uid::current())
            .context("determine current uid")?
            .ok_or_else(|| anyhow::anyhow!("couldn't resolve myself"))?;

        let mut cmd = Command::new(&path);
        cmd.args(["--policy", &args.policy_file, "--user", &user.name])
            .env(
                "KUMOD_LOG",
                "kumod=trace,kumo_server_common=info,kumo_server_runtime=info,amqprs=trace,warn,lua=trace",
            )
            .env("KUMOD_TEST_DIR", dir.path())
            .env("KUMO_NODE_ID_PATH", dir.path().join("nodeid"))
            .envs(args.env.iter().cloned())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);

        let cmd_label = format!("{cmd:?}");

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning {cmd_label}"))?;

        let mut stderr = BufReader::new(child.stderr.take().unwrap());

        // Send stdout to stderr
        let mut stdout = child.stdout.take().unwrap();

        async fn copy_stream_with_line_prefix<SRC, DEST>(
            prefix: &str,
            src: SRC,
            mut dest: DEST,
        ) -> std::io::Result<()>
        where
            SRC: AsyncRead + Unpin,
            DEST: AsyncWrite + Unpin,
        {
            let mut src = tokio::io::BufReader::new(src);
            loop {
                let mut line = String::new();
                src.read_line(&mut line).await?;
                if !line.is_empty() {
                    dest.write_all(format!("{prefix}: {line}").as_bytes())
                        .await?;
                }
            }
        }

        let stdout_prefix = format!("{} stdout", &args.policy_file);
        tokio::spawn(async move {
            copy_stream_with_line_prefix(&stdout_prefix, &mut stdout, &mut tokio::io::stderr())
                .await
        });

        // Wait until the server initializes, collect the information
        // about the various listeners that it starts
        let mut listeners = BTreeMap::new();
        loop {
            let mut line = String::new();
            stderr.read_line(&mut line).await?;
            if line.is_empty() {
                anyhow::bail!("Unexpected EOF while reading output from {cmd_label}");
            }
            eprintln!("{}", line.trim());

            if line.contains("initialization complete") {
                break;
            }

            if line.contains("listener on") {
                let mut fields: Vec<&str> = line.trim().split(' ').collect();
                while fields.len() > 4 {
                    fields.remove(0);
                }
                let proto = fields[0];
                let addr = fields[3];
                let addr: SocketAddr = addr.parse()?;
                listeners.insert(proto.to_string(), addr);
            }
        }

        // Now just pipe the output through to the test harness
        let stderr_prefix = format!("{} stderr", &args.policy_file);
        tokio::spawn(async move {
            copy_stream_with_line_prefix(&stderr_prefix, &mut stderr, &mut tokio::io::stderr())
                .await
        });

        Ok(Self {
            child,
            listeners,
            dir,
        })
    }

    pub async fn trace_server(&self) -> anyhow::Result<ServerTracer> {
        let (mut socket, _response) = connect_async(format!(
            "ws://{}/api/admin/trace-smtp-server/v1",
            self.listener("http")
        ))
        .await?;
        socket
            .send(Message::Text(
                serde_json::to_string(&TraceSmtpV1Request {
                    source_addr: None,
                    terse: true,
                })?
                .into(),
            ))
            .await?;

        let (signal_close, mut close_signalled) = tokio::sync::oneshot::channel();

        let records = Arc::new(Mutex::new(vec![]));

        let joiner = tokio::spawn({
            let records = records.clone();
            async move {
                let mut closed = false;

                while !socket.is_terminated() {
                    tokio::select! {
                        _ = &mut close_signalled, if !closed => {
                            closed = true;
                            if socket.close(None).await.is_err() {
                                break;
                            }
                        }
                        record = socket.next() => {
                            match record {
                                Some(Ok(msg)) => {
                                    match msg {
                                        Message::Text(s) => {
                                            let event: TraceSmtpV1Event = serde_json::from_str(&s).expect("TraceSmtpV1Event");
                                            records.lock().push(event);
                                        }
                                        Message::Ping(ping) => {
                                            if socket.send(Message::Pong(ping)).await.is_err() {
                                                break;
                                            }
                                        }
                                        wat => {
                                            eprintln!("WAT: {wat:?}");
                                            break;
                                        }
                                    }
                                }
                                Some(Err(_)) | None => {
                                    break;
                                }
                            }
                        }
                    };
                }
            }
        });

        Ok(ServerTracer {
            signal_close,
            joiner,
            records,
        })
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        let id = self.child.id().ok_or_else(|| anyhow::anyhow!("no pid!?"))?;
        let pid = nix::unistd::Pid::from_raw(id as _);
        nix::sys::signal::kill(pid, nix::sys::signal::SIGINT)?;
        tokio::select! {
            _ = self.child.wait() => Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                eprintln!("kumod didn't stop within 30 seconds");
                self.child.start_kill()?;
                Ok(())
            }
        }
    }

    pub fn listener(&self, service: &str) -> SocketAddr {
        match self.listeners.get(service) {
            Some(addr) => *addr,
            None => panic!("listener service {service} is not defined. Did it fail to start?"),
        }
    }

    pub fn api_client(&self) -> KumoApiClient {
        let endpoint = format!("http://{}", self.listener("http"));
        let url = kumo_api_client::Url::parse(&endpoint).unwrap();
        KumoApiClient::new(url)
    }

    pub async fn smtp_client(&self, host: &str) -> anyhow::Result<SmtpClient> {
        let mut client =
            SmtpClient::new(self.listener("smtp"), SmtpClientTimeouts::short_timeouts()).await?;

        let connect_timeout = client.timeouts().connect_timeout;
        let banner = client.read_response(None, connect_timeout).await?;
        anyhow::ensure!(banner.code == 220, "unexpected banner: {banner:#?}");
        client.ehlo(host).await?;
        Ok(client)
    }

    pub fn maildir(&self) -> Maildir {
        Maildir::with_path(self.dir.path().join("maildir"))
    }

    pub fn check_for_x_and_y_headers_in_logs(&self) -> anyhow::Result<()> {
        let dir = self.dir.path().join("logs");

        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let f = std::fs::File::open(entry.path())?;
                let data = zstd::stream::decode_all(f)?;
                let text = String::from_utf8(data)?;
                eprintln!("{text}");

                for line in text.lines() {
                    let record: JsonLogRecord = serde_json::from_str(line)?;
                    match record.kind {
                        RecordType::Reception | RecordType::Delivery => {
                            assert!(record.headers.contains_key("Subject"));
                            assert!(record.headers.contains_key("X-Test1"));
                            assert!(record.headers.contains_key("X-Another"));
                            assert!(!record.headers.contains_key("y-something"));
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn collect_acct_logs(&self) -> anyhow::Result<Vec<AcctLogRecord>> {
        let records = self.collect_acct_logs_impl().await?;

        // and print it in the sorted order for easier understanding
        eprintln!("--- collect_acct_logs begin");
        for r in &records {
            eprintln!("{}", serde_json::to_string(r).unwrap());
        }
        eprintln!("--- end of acct logs");

        Ok(records)
    }

    /// Raise an error if the acct log has any denied access entries.
    /// That implies that there is an issue with the ACL definitions
    pub async fn assert_no_acct_deny(&self) -> anyhow::Result<()> {
        let logs = self.collect_acct_logs().await?;

        for entry in &logs {
            anyhow::ensure!(entry.is_allow(), "Got a deny acct log entry: {entry:#?}");
        }
        Ok(())
    }

    pub async fn collect_acct_logs_impl(&self) -> anyhow::Result<Vec<AcctLogRecord>> {
        let dir = self.dir.path().join("acct");
        tokio::task::spawn_blocking(move || {
            let mut records = vec![];

            fn read_zstd_file(path: &Path) -> anyhow::Result<String> {
                let f = std::fs::File::open(path).with_context(|| format!("open {path:?}"))?;
                let data = zstd::stream::decode_all(f)
                    .with_context(|| format!("decoding zstd from {path:?}"))?;
                let text = String::from_utf8(data)?;
                Ok(text)
            }

            fn read_zstd_file_with_retry(path: &Path) -> anyhow::Result<String> {
                let mut error = None;
                for _ in 0..10 {
                    match read_zstd_file(path) {
                        Ok(t) => {
                            return Ok(t);
                        }
                        Err(err) => {
                            eprintln!("collect_logs: Error reading {path:?}: {err:#}");
                            error.replace(err);
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                    }
                }
                anyhow::bail!("Failed: {error:?}");
            }

            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    let text = read_zstd_file_with_retry(&entry.path())?;
                    for line in text.lines() {
                        let record: AcctLogRecord = serde_json::from_str(line)?;
                        records.push(record);
                    }
                }
            }

            records.sort_by(|a, b| a.timestamp().cmp(b.timestamp()));

            Ok(records)
        })
        .await?
    }

    pub async fn collect_logs(&self) -> anyhow::Result<Vec<JsonLogRecord>> {
        let records = self.collect_logs_impl().await?;

        // and print it in the sorted order for easier understanding
        eprintln!("--- collect_logs begin");
        for r in &records {
            eprintln!("{}", serde_json::to_string(r).unwrap());
        }
        eprintln!("--- end of logs");

        Ok(records)
    }

    pub async fn collect_logs_impl(&self) -> anyhow::Result<Vec<JsonLogRecord>> {
        let dir = self.dir.path().join("logs");
        tokio::task::spawn_blocking(move || {
            let mut records = vec![];

            fn read_zstd_file(path: &Path) -> anyhow::Result<String> {
                let f = std::fs::File::open(path).with_context(|| format!("open {path:?}"))?;
                let data = zstd::stream::decode_all(f)
                    .with_context(|| format!("decoding zstd from {path:?}"))?;
                let text = String::from_utf8(data)?;
                Ok(text)
            }

            fn read_zstd_file_with_retry(path: &Path) -> anyhow::Result<String> {
                let mut error = None;
                for _ in 0..10 {
                    match read_zstd_file(path) {
                        Ok(t) => {
                            return Ok(t);
                        }
                        Err(err) => {
                            eprintln!("collect_logs: Error reading {path:?}: {err:#}");
                            error.replace(err);
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                    }
                }
                anyhow::bail!("Failed: {error:?}");
            }

            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    let text = read_zstd_file_with_retry(&entry.path())?;
                    for line in text.lines() {
                        let record: JsonLogRecord = serde_json::from_str(line)?;
                        records.push(record);
                    }
                }
            }

            records.sort_by(|a, b| {
                use std::cmp::Ordering;
                match a.timestamp.cmp(&b.timestamp) {
                    Ordering::Equal => match a.id.cmp(&b.id) {
                        Ordering::Equal => a.kind.cmp(&b.kind),
                        r => r,
                    },
                    r => r,
                }
            });

            Ok(records)
        })
        .await?
    }

    pub async fn dump_logs(&self) -> anyhow::Result<BTreeMap<RecordType, usize>> {
        let mut counts = BTreeMap::new();
        for record in self.collect_logs().await? {
            *counts.entry(record.kind).or_default() += 1;
        }
        Ok(counts)
    }

    pub async fn summarize_logs(&self) -> anyhow::Result<BTreeMap<RecordType, usize>> {
        let mut counts = BTreeMap::new();
        for record in self.collect_logs_impl().await? {
            *counts.entry(record.kind).or_default() += 1;
        }
        Ok(counts)
    }

    pub async fn wait_for_maildir_count(&self, count: usize, timeout: Duration) -> bool {
        eprintln!("waiting for maildir to populate");
        let md = self.maildir();

        tokio::select! {
            _ = async {
                    while md.count_new() != count {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
            } => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }

    pub fn accounting_stats(&self) -> anyhow::Result<AccountingStats> {
        let path = self.dir.path().join("accounting.db");

        let mut db = Connection::open_thread_safe(&path)
            .with_context(|| format!("opening accounting database {path:?}"))?;
        db.set_busy_timeout(30_000)?;

        let mut stmt = db
            .prepare("select sum(received) as r, sum(delivered) as d from accounting")
            .with_context(|| format!("prepare query against {path:?}"))?;
        if let Ok(State::Row) = stmt.next() {
            let received = stmt.read::<i64, _>("r")?;
            let delivered = stmt.read::<i64, _>("d")?;

            return Ok(AccountingStats {
                received: received as usize,
                delivered: delivered as usize,
            });
        }

        anyhow::bail!("unexpected state from accounting db");
    }

    pub async fn kcli_text(
        &self,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> anyhow::Result<String> {
        let path = target_bin("kcli")?;
        let mut cmd = Command::new(path);
        cmd.args(["--endpoint", &format!("http://{}", self.listener("http"))]);
        cmd.args(args);
        cmd.stdout(std::process::Stdio::piped());
        let label = format!("{cmd:?}");
        let child = cmd.spawn()?;
        let output = child.wait_with_output().await?;
        anyhow::ensure!(output.status.success(), "{label}: {:?}", output.status);
        let output = String::from_utf8_lossy(&output.stdout);
        println!("kcli output is: {output}");
        Ok(output.into())
    }

    pub async fn kcli_json<R: for<'a> serde::Deserialize<'a>>(
        &self,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> anyhow::Result<R> {
        let path = target_bin("kcli")?;
        let mut cmd = Command::new(path);
        cmd.args(["--endpoint", &format!("http://{}", self.listener("http"))]);
        cmd.args(args);
        cmd.stdout(std::process::Stdio::piped());
        let label = format!("{cmd:?}");
        let child = cmd.spawn()?;
        let output = child.wait_with_output().await?;
        anyhow::ensure!(output.status.success(), "{label}: {:?}", output.status);
        println!(
            "kcli output is: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}

#[derive(Clone, Debug, Default)]
#[allow(unused)]
pub struct AccountingStats {
    pub received: usize,
    pub delivered: usize,
}

pub struct DaemonWithMaildirAndWebHook {
    pub with_maildir: DaemonWithMaildir,
    pub webhook: WebHookServer,
}

pub struct BatchParams {
    pub min_batch_size: usize,
    pub max_batch_size: usize,
    pub max_batch_latency: usize,
}

impl DaemonWithMaildirAndWebHook {
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_batched(BatchParams {
            min_batch_size: 1,
            max_batch_size: 1,
            max_batch_latency: 0,
        })
        .await
    }

    pub async fn start_batched(batch_params: BatchParams) -> anyhow::Result<Self> {
        let webhook = WebHookServer::start().await?;
        let sink = KumoDaemon::spawn_maildir().await?;
        let smtp = sink.listener("smtp");
        let source = KumoDaemon::spawn(KumoArgs {
            policy_file: "source.lua".to_string(),
            env: vec![
                ("KUMOD_SMTP_SINK_PORT".to_string(), smtp.port().to_string()),
                (
                    "KUMOD_WEBHOOK_PORT".to_string(),
                    webhook.addr.port().to_string(),
                ),
                (
                    "KUMOD_WEBHOOK_MAX_BATCH_SIZE".to_string(),
                    batch_params.max_batch_size.to_string(),
                ),
                (
                    "KUMOD_WEBHOOK_MIN_BATCH_SIZE".to_string(),
                    batch_params.min_batch_size.to_string(),
                ),
                (
                    "KUMOD_WEBHOOK_MAX_BATCH_LATENCY".to_string(),
                    format!("{}s", batch_params.max_batch_latency),
                ),
            ],
        })
        .await?;

        Ok(Self {
            with_maildir: DaemonWithMaildir { source, sink },
            webhook,
        })
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        self.with_maildir.stop_both().await?;
        self.webhook.shutdown();
        Ok(())
    }

    pub async fn wait_for_webhook_record_count(&self, count: usize, timeout: Duration) -> bool {
        self.webhook.wait_for_record_count(count, timeout).await
    }
}

pub struct DaemonWithTsa {
    pub with_maildir: DaemonWithMaildir,
    pub tsa: TsaDaemon,
}

impl DaemonWithTsa {
    pub async fn start() -> anyhow::Result<Self> {
        let tsa = TsaDaemon::spawn(TsaArgs {
            policy_file: "tsa_init.lua".to_string(),
            env: vec![],
        })
        .await?;
        let tsa_listener = tsa.listener("http");

        let sink = KumoDaemon::spawn_maildir().await?;
        let smtp = sink.listener("smtp");
        let source = KumoDaemon::spawn(KumoArgs {
            policy_file: "tsa_source.lua".to_string(),
            env: vec![
                ("KUMOD_SMTP_SINK_PORT".to_string(), smtp.port().to_string()),
                (
                    "KUMOD_TSA_PORT".to_string(),
                    tsa_listener.port().to_string(),
                ),
            ],
        })
        .await?;

        Ok(Self {
            with_maildir: DaemonWithMaildir { source, sink },
            tsa,
        })
    }

    pub async fn smtp_client(&self) -> anyhow::Result<SmtpClient> {
        self.with_maildir.source.smtp_client("localhost").await
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        self.with_maildir.stop_both().await?;
        self.tsa.stop().await?;
        Ok(())
    }
}

pub struct ServerTracer {
    signal_close: tokio::sync::oneshot::Sender<()>,
    joiner: tokio::task::JoinHandle<()>,
    records: Arc<Mutex<Vec<TraceSmtpV1Event>>>,
}

impl ServerTracer {
    pub async fn stop(self) -> anyhow::Result<Vec<TraceSmtpV1Event>> {
        self.signal_close.send(()).expect("call stop only once");
        self.joiner.await?;
        Ok(self.records.lock().clone())
    }

    pub fn with_records<R>(&self, mut apply: impl FnMut(&[TraceSmtpV1Event]) -> R) -> R {
        let records = self.records.lock();
        (apply)(&records)
    }

    pub async fn wait_for(
        &self,
        mut condition: impl FnMut(&[TraceSmtpV1Event]) -> bool,
        timeout: Duration,
    ) -> bool {
        tokio::select! {
            _ = async {
                loop {
                    if self.with_records(&mut condition) {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }
}
