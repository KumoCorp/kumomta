use anyhow::Context;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug)]
pub struct TsaDaemon {
    pub dir: TempDir,
    pub listeners: BTreeMap<String, SocketAddr>,
    child: Child,
}

#[derive(Default, Debug)]
pub struct TsaArgs {
    pub policy_file: String,
    pub env: Vec<(String, String)>,
}

impl TsaDaemon {
    pub async fn spawn(args: TsaArgs) -> anyhow::Result<Self> {
        let path = if cfg!(debug_assertions) {
            "../../target/debug/tsa-daemon"
        } else {
            "../../target/release/tsa-daemon"
        };
        let path = std::fs::canonicalize(path).with_context(|| format!("canonicalize {path}"))?;

        let dir = tempfile::tempdir().context("make temp dir")?;

        let mut cmd = Command::new(&path);
        cmd.args(["--policy", &args.policy_file])
            .env("KUMO_TSA_LOG", "tsa_daemon=trace,kumo_server_common=info")
            .env("KUMO_TSA_TEST_DIR", dir.path())
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
}
