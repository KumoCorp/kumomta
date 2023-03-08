use anyhow::Context;
use maildir::Maildir;
use rfc5321::{SmtpClient, SmtpClientTimeouts};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug)]
pub struct KumoDaemon {
    pub dir: TempDir,
    pub listeners: HashMap<String, SocketAddr>,
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
        let mut listeners = HashMap::new();
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
            SmtpClient::new(self.listener("smtp"), SmtpClientTimeouts::default()).await?;

        let banner = client.read_response(None).await?;
        anyhow::ensure!(banner.code == 220, "unexpected banner: {banner:#?}");
        client.ehlo("localhost").await?;
        Ok(client)
    }

    pub fn maildir(&self) -> Maildir {
        Maildir::from(self.dir.path().join("maildir"))
    }

    pub fn dump_logs(&self) -> anyhow::Result<()> {
        let dir = self.dir.path().join("logs");
        for entry in std::fs::read_dir(&dir)? {
            let entry = dbg!(entry)?;
            if entry.file_type()?.is_file() {
                let f = std::fs::File::open(entry.path())?;
                let data = zstd::stream::decode_all(f)?;
                let text = String::from_utf8(data)?;
                eprintln!("{text}");
            }
        }
        Ok(())
    }

    pub async fn wait_for_maildir_count(&self, count: usize, timeout: Duration) -> bool {
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
