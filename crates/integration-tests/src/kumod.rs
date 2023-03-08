use anyhow::Context;
use kumo_log_types::*;
use maildir::{MailEntry, Maildir};
use rfc5321::{ForwardPath, Response, ReversePath, SmtpClient, SmtpClientTimeouts};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
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
        eprintln!("using {body}");
        let message = mail_builder::MessageBuilder::new()
            .from(sender)
            .to(recip)
            .subject(self.subject.unwrap_or("Hello! This is a test"))
            .text_body(body)
            .write_to_string()?;
        Ok(message)
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
        let sink = KumoDaemon::spawn_maildir().await?;
        let smtp = sink.listener("smtp");
        let source = KumoDaemon::spawn(KumoArgs {
            policy_file: "source.lua".to_string(),
            env: vec![("KUMOD_SMTP_SINK_PORT".to_string(), smtp.port().to_string())],
        })
        .await?;

        Ok(Self { source, sink })
    }

    pub async fn smtp_client(&self) -> anyhow::Result<SmtpClient> {
        self.source.smtp_client().await
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

        let mut child = Command::new(&path)
            .args(["--policy", &args.policy_file])
            .env("KUMOD_LOG", "kumod=trace")
            .env("KUMOD_TEST_DIR", dir.path())
            .envs(args.env.iter().cloned())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawning {}", path.display()))?;

        let mut stderr = BufReader::new(child.stderr.take().unwrap());

        // Send stdout to stderr
        let mut stdout = child.stdout.take().unwrap();
        tokio::spawn(async move { tokio::io::copy(&mut stdout, &mut tokio::io::stderr()).await });

        // Wait until the server initializes, collect the information
        // about the various listeners that it starts
        let mut listeners = BTreeMap::new();
        loop {
            let mut line = String::new();
            stderr.read_line(&mut line).await?;
            if line.is_empty() {
                anyhow::bail!("Unexpected EOF");
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
        tokio::spawn(async move { tokio::io::copy(&mut stderr, &mut tokio::io::stderr()).await });

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
        self.listeners.get(service).copied().unwrap()
    }

    pub async fn smtp_client(&self) -> anyhow::Result<SmtpClient> {
        let mut client =
            SmtpClient::new(self.listener("smtp"), SmtpClientTimeouts::short_timeouts()).await?;

        let banner = client.read_response(None).await?;
        anyhow::ensure!(banner.code == 220, "unexpected banner: {banner:#?}");
        client.ehlo("localhost").await?;
        Ok(client)
    }

    pub fn maildir(&self) -> Maildir {
        Maildir::from(self.dir.path().join("maildir"))
    }

    pub fn dump_logs(&self) -> anyhow::Result<BTreeMap<RecordType, usize>> {
        let dir = self.dir.path().join("logs");
        let mut counts = BTreeMap::new();

        for entry in std::fs::read_dir(&dir)? {
            let entry = dbg!(entry)?;
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
}
