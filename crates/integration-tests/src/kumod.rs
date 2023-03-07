use anyhow::Context;
use std::collections::HashMap;
use std::process::Stdio;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug)]
pub struct KumoDaemon {
    pub dir: TempDir,
    pub listeners: HashMap<String, String>,
    child: Child,
}

#[derive(Default, Debug)]
pub struct KumoArgs {
    pub policy_file: String,
    pub env: Vec<(String, String)>,
}

impl KumoDaemon {
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
            .env("KUMOD_LOG", "kumod=info")
            .env("KUMOD_TEST_DIR", dir.path())
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawning {}", path.display()))?;

        let mut stderr = BufReader::new(child.stderr.take().unwrap());
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
                listeners.insert(proto.to_string(), addr.to_string());
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
        self.child.wait().await?;
        Ok(())
    }
}
