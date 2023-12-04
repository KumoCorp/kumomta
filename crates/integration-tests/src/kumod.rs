use crate::webhook::WebHookServer;
use anyhow::Context;
use kumo_log_types::*;
use maildir::{MailEntry, Maildir};
use mailparsing::MessageBuilder;
use nix::unistd::{Uid, User};
use rfc5321::{ForwardPath, Response, ReversePath, SmtpClient, SmtpClientTimeouts};
use sqlite::{Connection, State};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Default, Clone)]
pub struct MailGenParams<'a> {
    pub sender: Option<&'a str>,
    pub recip: Option<&'a str>,
    pub size: Option<usize>,
    pub wrap: Option<usize>,
    pub subject: Option<&'a str>,
    pub body: Option<&'a str>,
}

/// Generate a single nonsense string with no spaces with
/// length at least that specified
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
    pub async fn send(&self, client: &mut SmtpClient) -> anyhow::Result<Response> {
        let sender = self.sender.unwrap_or("sender@example.com");
        let recip = self.recip.unwrap_or("recip@example.com");
        let body = self.generate()?;

        Ok(client
            .send_mail(
                ReversePath::try_from(sender).unwrap(),
                ForwardPath::try_from(recip).unwrap(),
                &body,
            )
            .await?)
    }

    pub fn generate(&self) -> anyhow::Result<String> {
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
        let recip = self.recip.unwrap_or("recip@example.com");
        let mut message = MessageBuilder::new();
        message.set_from(sender);
        message.set_to(recip);
        message.set_subject(self.subject.unwrap_or("Hello! This is a test"));
        message.text_plain(body);
        message.prepend("X-Test1", "Test1");
        message.prepend("X-Another", "Another");
        Ok(message.build()?.to_message_string())
    }
}

#[derive(Debug)]
pub struct DeliverySummary {
    pub source_counts: BTreeMap<RecordType, usize>,
    pub sink_counts: BTreeMap<RecordType, usize>,
}

pub struct DaemonWithMaildir {
    pub source: KumoDaemon,
    pub sink: KumoDaemon,
}

impl DaemonWithMaildir {
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with_env(vec![]).await
    }

    pub async fn start_with_env(env: Vec<(&str, &str)>) -> anyhow::Result<Self> {
        let sink = KumoDaemon::spawn_maildir().await.context("spawn_maildir")?;
        let smtp = sink.listener("smtp");

        let mut env: Vec<(String, String)> = env
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        env.push(("KUMOD_SMTP_SINK_PORT".to_string(), smtp.port().to_string()));

        let source = KumoDaemon::spawn(KumoArgs {
            policy_file: "source.lua".to_string(),
            env,
        })
        .await
        .context("KumoDaemon::spawn")?;

        Ok(Self { source, sink })
    }

    pub async fn smtp_client(&self) -> anyhow::Result<SmtpClient> {
        self.source.smtp_client().await
    }

    pub async fn kcli(
        &self,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> anyhow::Result<std::process::ExitStatus> {
        let path = if cfg!(debug_assertions) {
            "../../target/debug/kcli"
        } else {
            "../../target/release/kcli"
        };
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

    pub async fn kcli_json<R: for<'a> serde::Deserialize<'a>>(
        &self,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> anyhow::Result<R> {
        let path = if cfg!(debug_assertions) {
            "../../target/debug/kcli"
        } else {
            "../../target/release/kcli"
        };
        let mut cmd = Command::new(path);
        cmd.args([
            "--endpoint",
            &format!("http://{}", self.source.listener("http")),
        ]);
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
                    if let Ok(summary) = self.source.dump_logs() {
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
        Ok(())
    }

    pub fn dump_logs(&self) -> anyhow::Result<DeliverySummary> {
        eprintln!("source logs:");
        let source_counts = self.source.dump_logs()?;
        eprintln!("sink logs:");
        let sink_counts = self.sink.dump_logs()?;
        Ok(DeliverySummary {
            source_counts,
            sink_counts,
        })
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
        KumoDaemon::spawn(KumoArgs {
            policy_file: "maildir-sink.lua".to_string(),
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
        let path = if cfg!(debug_assertions) {
            "../../target/debug/kumod"
        } else {
            "../../target/release/kumod"
        };
        let path = std::fs::canonicalize(path).with_context(|| format!("canonicalize {path}"))?;

        let dir = tempfile::tempdir().context("make temp dir")?;

        let user = User::from_uid(Uid::current())
            .context("determine current uid")?
            .ok_or_else(|| anyhow::anyhow!("couldn't resolve myself"))?;

        let mut cmd = Command::new(&path);
        cmd.args(["--policy", &args.policy_file, "--user", &user.name])
            .env("KUMOD_LOG", "kumod=trace,kumo_server_common=info")
            .env("KUMOD_TEST_DIR", dir.path())
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

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        let id = self.child.id().ok_or_else(|| anyhow::anyhow!("no pid!?"))?;
        let pid = nix::unistd::Pid::from_raw(id as _);
        nix::sys::signal::kill(pid, nix::sys::signal::SIGINT)?;
        tokio::select! {
            _ = self.child.wait() => Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(10)) => {
                eprintln!("daemon didn't stop within 10 seconds");
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

    pub async fn smtp_client(&self) -> anyhow::Result<SmtpClient> {
        let mut client =
            SmtpClient::new(self.listener("smtp"), SmtpClientTimeouts::short_timeouts()).await?;

        let connect_timeout = client.timeouts().connect_timeout;
        let banner = client.read_response(None, connect_timeout).await?;
        anyhow::ensure!(banner.code == 220, "unexpected banner: {banner:#?}");
        client.ehlo("localhost").await?;
        Ok(client)
    }

    pub fn maildir(&self) -> Maildir {
        Maildir::from(self.dir.path().join("maildir"))
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
                    let record: JsonLogRecord = serde_json::from_str(&line)?;
                    if record.kind == RecordType::Reception {
                        assert!(record.headers.contains_key("X-Test1"));
                        assert!(record.headers.contains_key("X-Another"));
                        assert!(!record.headers.contains_key("y-something"));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn dump_logs(&self) -> anyhow::Result<BTreeMap<RecordType, usize>> {
        let dir = self.dir.path().join("logs");
        let mut counts = BTreeMap::new();

        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let f = std::fs::File::open(entry.path())?;
                let data = zstd::stream::decode_all(f)?;
                let text = String::from_utf8(data)?;
                eprintln!("{text}");

                for line in text.lines() {
                    let record: JsonLogRecord = serde_json::from_str(&line)?;
                    *counts.entry(record.kind).or_default() += 1;
                }
            }
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

        let db = Connection::open_with_full_mutex(&path)
            .with_context(|| format!("opening accounting database {path:?}"))?;

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
}

#[derive(Clone, Debug, Default)]
pub struct AccountingStats {
    pub received: usize,
    pub delivered: usize,
}

pub struct DaemonWithMaildirAndWebHook {
    pub with_maildir: DaemonWithMaildir,
    pub webhook: WebHookServer,
}

impl DaemonWithMaildirAndWebHook {
    pub async fn start() -> anyhow::Result<Self> {
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
