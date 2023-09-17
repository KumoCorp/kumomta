use crate::{NodeSpec, RedisConnKey, RedisConnection};
use anyhow::Context;
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

/// A local redis server for executing tests against
pub struct RedisServer {
    _daemon: Child,
    port: u16,
    _dir: TempDir,
}

/// Ask the kernel to assign a free port.
/// We pass this to sshd and tell it to listen on that port.
/// This is racy, as releasing the socket technically makes
/// that port available to others using the same technique.
fn allocate_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0 failed");
    listener.local_addr().unwrap().port()
}

impl RedisServer {
    pub async fn spawn() -> anyhow::Result<Self> {
        let mut errors = vec![];

        for _ in 0..2 {
            let port = allocate_port();
            match timeout(Duration::from_secs(5), Self::spawn_with_port(port)).await? {
                Ok(me) => return Ok(me),
                Err(err) => {
                    errors.push(format!("{err:#}"));
                }
            }
        }
        anyhow::bail!("failed to spawn redis-server: {}", errors.join(". "));
    }

    async fn spawn_with_port(port: u16) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir().context("make temp dir")?;
        let mut daemon = Command::new("redis-server")
            .args(["-"])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("spawning redis-server")?;

        let mut stdout = BufReader::new(daemon.stdout.take().unwrap());
        let mut stderr = daemon.stderr.take().unwrap();

        tokio::spawn(async move {
            copy_stream_with_line_prefix("redis stderr", &mut stderr, &mut tokio::io::stderr())
                .await
        });

        // Generate configuration
        if let Some(mut stdin) = daemon.stdin.take() {
            stdin
                .write_all(b"bind 127.0.0.1\nlogfile /dev/stdout\n")
                .await?;
            stdin.write_all(format!("port {port}\n").as_bytes()).await?;
            stdin
                .write_all(format!("dir {}\n", dir.path().display()).as_bytes())
                .await?;
            drop(stdin);
        }

        // Wait until the server initializes successfully
        loop {
            let mut line = String::new();
            stdout.read_line(&mut line).await?;
            if line.is_empty() {
                anyhow::bail!("Unexpected EOF while reading output from redis-server");
            }
            eprintln!("{}", line.trim());

            if line.contains("Server initialized")
                || line.contains("The server is now ready to accept connections on port")
            {
                break;
            }
        }

        // Now just pipe the output through to the test harness
        tokio::spawn(async move {
            copy_stream_with_line_prefix("redis stdout", &mut stdout, &mut tokio::io::stderr())
                .await
        });

        Ok(Self {
            _daemon: daemon,
            port,
            _dir: dir,
        })
    }

    pub async fn connection(&self) -> anyhow::Result<RedisConnection> {
        let key = RedisConnKey {
            node: NodeSpec::Single(format!("redis://127.0.0.1:{}", self.port)),
            read_from_replicas: false,
            username: None,
            password: None,
            pool_size: None,
        };
        key.open().await
    }
}

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

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_basic_operation() -> anyhow::Result<()> {
        if which::which("redis-server").is_err() {
            return Ok(());
        }
        let daemon = RedisServer::spawn().await?;
        let connection = daemon.connection().await?;

        let mut cmd = redis::cmd("SET");
        cmd.arg("my_key").arg(42);
        connection.query(cmd).await?;

        let mut cmd = redis::cmd("GET");
        cmd.arg("my_key");
        let value = connection.query(cmd).await?;

        assert_eq!(value, redis::Value::Data(b"42".to_vec()));

        Ok(())
    }
}
