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
    pub fn is_available() -> bool {
        which::which("redis-server").is_ok()
    }

    pub async fn spawn(extra_config: &str) -> anyhow::Result<Self> {
        let mut errors = vec![];

        for _ in 0..2 {
            let port = allocate_port();
            match timeout(
                Duration::from_secs(5),
                Self::spawn_with_port(port, extra_config),
            )
            .await?
            {
                Ok(me) => return Ok(me),
                Err(err) => {
                    errors.push(format!("{err:#}"));
                }
            }
        }
        anyhow::bail!("failed to spawn redis-server: {}", errors.join(". "));
    }

    async fn spawn_with_port(port: u16, extra_config: &str) -> anyhow::Result<Self> {
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
                .write_all(b"bind 127.0.0.1\nlogfile /dev/stdout\nloglevel debug\n")
                .await?;
            stdin.write_all(format!("port {port}\n").as_bytes()).await?;
            stdin
                .write_all(format!("dir {}\n", dir.path().display()).as_bytes())
                .await?;
            stdin
                .write_all(format!("{extra_config}\n").as_bytes())
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
            cluster: None,
            pool_size: None,
            connect_timeout: None,
            recycle_timeout: None,
            wait_timeout: None,
            response_timeout: None,
        };
        key.open()
    }
}

pub struct RedisCluster {
    primary: RedisServer,
    secondary: RedisServer,
    tertiary: RedisServer,
}

impl RedisCluster {
    /// Check whether redis is available to run as a cluster.
    /// We look for redis 7.x and later, because we rely on
    /// the --cluster-yes option actually working as part of
    /// our cluster initialization. It doesn't work on redis 5.x
    /// which is present on rocky8 for example.
    pub async fn is_available() -> bool {
        if !RedisServer::is_available() {
            return false;
        }

        match Command::new("redis-cli").arg("-v").output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                match stdout.lines().next() {
                    Some(line) => {
                        let Some((redis, version)) = line.split_once(" ") else {
                            return false;
                        };
                        if redis == "redis-cli" {
                            let Some((major, _rest)) = version.split_once(".") else {
                                return false;
                            };
                            let Ok(major) = major.parse::<u32>() else {
                                return false;
                            };
                            major >= 7
                        } else {
                            false
                        }
                    }
                    None => false,
                }
            }
            Err(_) => false,
        }
    }

    pub async fn spawn() -> anyhow::Result<Self> {
        let extra_config = "cluster-enabled yes\n";
        let primary = RedisServer::spawn(extra_config).await?;
        let secondary = RedisServer::spawn(extra_config).await?;
        let tertiary = RedisServer::spawn(extra_config).await?;

        let cluster_setup = Command::new("redis-cli")
            .args([
                "--cluster",
                "create",
                &format!("127.0.0.1:{}", primary.port),
                &format!("127.0.0.1:{}", secondary.port),
                &format!("127.0.0.1:{}", tertiary.port),
                "--cluster-yes",
            ])
            .kill_on_drop(true)
            .output()
            .await
            .context("create redis cluster")?;

        if !cluster_setup.stdout.is_empty() {
            eprintln!(
                "cluster_setup stdout: {}",
                String::from_utf8_lossy(&cluster_setup.stdout)
            );
        }
        if !cluster_setup.stderr.is_empty() {
            eprintln!(
                "cluster_setup stderr: {}",
                String::from_utf8_lossy(&cluster_setup.stderr)
            );
        }

        Ok(Self {
            primary,
            secondary,
            tertiary,
        })
    }

    pub async fn connection(&self) -> anyhow::Result<RedisConnection> {
        let key = RedisConnKey {
            node: NodeSpec::Cluster(vec![
                format!("redis://127.0.0.1:{}", self.primary.port),
                format!("redis://127.0.0.1:{}", self.secondary.port),
                format!("redis://127.0.0.1:{}", self.tertiary.port),
            ]),
            read_from_replicas: false,
            username: None,
            password: None,
            cluster: None,
            pool_size: None,
            connect_timeout: None,
            recycle_timeout: None,
            wait_timeout: None,
            response_timeout: None,
        };
        key.open()
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

    fn config(node: impl serde::Serialize) -> RedisConnKey {
        serde_json::from_value(serde_json::json!({"node": node})).unwrap()
    }

    #[test]
    fn labels_keep_legacy_spelling_order_and_checksum() {
        use std::hash::Hash;
        let plain = "REDIS://Cache.Example:6379/0?protocol=resp2";
        let auth = "redis://user:p%40ss%3Aword@a/2";
        for (mut key, nodes) in [
            (config(plain), plain.to_owned()),
            (
                config("rediss://:pw@[::1]:6380/1?protocol=resp3"),
                "rediss://:[REDACTED]@[::1]:6380/1?protocol=resp3".to_owned(),
            ),
            (
                config([plain, auth, plain]),
                format!("{plain},redis://user:[REDACTED]@a/2,{plain}"),
            ),
        ] {
            key.username = Some("pool-user".into());
            key.password = Some("separate-password".into());
            let mut crc = crc32fast::Hasher::new();
            key.hash(&mut crc);
            assert_eq!(
                key.hash_label(),
                format!("pool-user@{nodes}-{:08x}", crc.finalize())
            );
        }
    }

    #[test]
    fn debug_distinguishes_configured_and_absent_passwords() {
        let mut key = config("redis://alice:node-secret@cache.example:6379/2");
        key.pool_size = Some(8);
        assert!(format!("{key:?}").contains("password: None"));
        key.password = Some("separate-secret".into());
        let debug = format!("{:?}", key.open().unwrap());
        assert!(debug.contains(r#"password: Some("[REDACTED]")"#));
        assert!(debug.contains("redis://alice:[REDACTED]@cache.example:6379/2"));
        assert!(debug.contains("pool_size: Some(8)"));
        assert!(!debug.contains("node-secret") && !debug.contains("separate-secret"));
    }

    #[test]
    fn constructor_error_keeps_cause_but_omits_passwords() {
        let bad = "redis://u:url-password@cache.example/invalid-db";
        for mut key in [config(bad), config(["redis://b", bad])] {
            key.password = Some("separate-password".into());
            let err = format!("{:#}", key.open().unwrap_err());
            assert!(err.contains("cache.example") && err.contains("Invalid database number"));
            assert!(!err.contains("url-password") && !err.contains("separate-password"));
        }
    }

    #[tokio::test]
    async fn test_basic_operation() -> anyhow::Result<()> {
        if !RedisServer::is_available() {
            return Ok(());
        }
        let daemon = RedisServer::spawn("").await?;
        let connection = daemon.connection().await?;

        let mut cmd = redis::cmd("SET");
        cmd.arg("my_key").arg(42);
        connection.query(cmd).await?;

        let mut cmd = redis::cmd("GET");
        cmd.arg("my_key");
        let value = connection.query(cmd).await?;

        assert_eq!(value, redis::Value::BulkString(b"42".to_vec()));

        Ok(())
    }

    #[tokio::test]
    async fn test_basic_operation_cluster() -> anyhow::Result<()> {
        if !RedisCluster::is_available().await {
            return Ok(());
        }
        let daemon = RedisCluster::spawn().await?;
        let connection = daemon.connection().await?;

        let mut cmd = redis::cmd("SET");
        cmd.arg("my_key").arg(42);
        connection.query(cmd).await?;

        let mut cmd = redis::cmd("GET");
        cmd.arg("my_key");
        let value = connection.query(cmd).await?;

        assert_eq!(value, redis::Value::BulkString(b"42".to_vec()));

        Ok(())
    }
}
