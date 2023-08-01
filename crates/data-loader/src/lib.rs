use anyhow::{anyhow, Context};
use config::{any_err, from_lua_value, get_or_create_sub_module};
use mlua::Lua;
use serde::Deserialize;
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};

#[derive(Deserialize, Clone, Hash, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum KeySource {
    File(String),
    Data {
        key_data: String,
    },
    Vault {
        vault_address: Option<String>,
        vault_token: Option<String>,
        vault_mount: String,
        vault_path: String,
    },
}

impl KeySource {
    pub async fn get(&self) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::File(path) => Ok(tokio::fs::read(path).await?),
            Self::Data { key_data } => Ok(key_data.as_bytes().to_vec()),
            Self::Vault {
                vault_address,
                vault_token,
                vault_mount,
                vault_path,
            } => {
                let address = match vault_address {
                    Some(a) => a.to_string(),
                    None => std::env::var("VAULT_ADDR").map_err(|err| {
                        anyhow!(
                            "address was not specified and $VAULT_ADDR is not set/usable: {err:#}"
                        )
                    })?,
                };
                let token = match vault_token {
                    Some(a) => a.to_string(),
                    None => std::env::var("VAULT_TOKEN").map_err(|err| {
                        anyhow!(
                            "address was not specified and $VAULT_TOKEN is not set/usable: {err:#}"
                        )
                    })?,
                };

                let client = VaultClient::new(
                    VaultClientSettingsBuilder::default()
                        .address(address)
                        .token(token)
                        .build()?,
                )?;

                #[derive(Deserialize, Debug)]
                struct Entry {
                    key: String,
                }

                let entry: Entry = vaultrs::kv2::read(&client, vault_mount, vault_path)
                    .await
                    .with_context(|| {
                        format!("kv2::read vault_mount={vault_mount}, vault_path={vault_path}")
                    })?;

                Ok(entry.key.into())
            }
        }
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let secrets_mod = get_or_create_sub_module(lua, "secrets")?;

    secrets_mod.set(
        "load",
        lua.create_async_function(|lua, source: mlua::Value| async move {
            let source: KeySource = from_lua_value(lua, source)?;
            source.get().await.map_err(any_err)
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use anyhow::Context;
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt};
    use tokio::process::{Child, Command};
    use tokio::time::timeout;
    use vaultrs::client::Client;

    /// Ask the kernel to assign a free port.
    /// We pass this to sshd and tell it to listen on that port.
    /// This is racy, as releasing the socket technically makes
    /// that port available to others using the same technique.
    fn allocate_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0 failed");
        listener.local_addr().unwrap().port()
    }

    const KEY: &str = "woot";

    struct VaultServer {
        port: u16,
        _daemon: Child,
    }

    impl VaultServer {
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
            anyhow::bail!("failed to spawn vault: {}", errors.join(". "));
        }

        async fn spawn_with_port(port: u16) -> anyhow::Result<Self> {
            eprintln!("Trying to start vault on port {port}");

            let mut daemon = Command::new("vault")
                .args([
                    "server",
                    "-dev",
                    &format!("-dev-listen-address=127.0.0.1:{port}"),
                    &format!("-dev-root-token-id={KEY}"),
                ])
                .stdin(Stdio::piped())
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .context("spawning vault")?;

            let mut stderr = daemon.stderr.take().unwrap();
            tokio::spawn(async move {
                copy_stream_with_line_prefix("vault stderr", &mut stderr, &mut tokio::io::stderr())
                    .await
            });
            let mut stdout = daemon.stdout.take().unwrap();
            tokio::spawn(async move {
                copy_stream_with_line_prefix("vault stdout", &mut stdout, &mut tokio::io::stderr())
                    .await
            });

            let mut ok = false;
            for _ in 0..25 {
                let client = VaultClient::new(
                    VaultClientSettingsBuilder::default()
                        .address(format!("http://127.0.0.1:{port}"))
                        .token(KEY)
                        .build()?,
                )?;
                let status = client.status().await;
                eprintln!("checking status: {status:?}");
                if let Ok(vaultrs::sys::ServerStatus::OK) = status {
                    ok = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            anyhow::ensure!(ok, "server didn't startup successfully");

            if let Ok(Some(status)) = daemon.try_wait() {
                anyhow::bail!("daemon exited already: {status:?}");
            }

            Ok(Self {
                _daemon: daemon,
                port,
            })
        }

        pub async fn put_from_file(&self, vault_path: &str, path: &str) -> anyhow::Result<()> {
            let output = Command::new("vault")
                .args([
                    "kv",
                    "put",
                    &format!("-address=http://127.0.0.1:{}", self.port),
                    "-mount=secret",
                    vault_path,
                    &format!("key=@{path}"),
                ])
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.is_empty() {
                eprintln!("put_from_file: {stdout}");
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                eprintln!("put_from_file: {stderr}");
            }
            anyhow::ensure!(output.status.success(), "{:?}", output.status);
            Ok(())
        }

        pub async fn put(&self, vault_path: &str, value: &str) -> anyhow::Result<()> {
            let output = Command::new("vault")
                .args([
                    "kv",
                    "put",
                    &format!("-address=http://127.0.0.1:{}", self.port),
                    "-mount=secret",
                    vault_path,
                    &format!("key={value}"),
                ])
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.is_empty() {
                eprintln!("put: {stdout}");
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                eprintln!("put: {stderr}");
            }
            anyhow::ensure!(output.status.success(), "{:?}", output.status);
            Ok(())
        }

        pub fn make_source(&self, path: &str) -> KeySource {
            KeySource::Vault {
                vault_address: Some(format!("http://127.0.0.1:{}", self.port)),
                vault_token: Some(KEY.to_string()),
                vault_mount: "secret".to_string(),
                vault_path: path.to_string(),
            }
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

    #[tokio::test]
    async fn test_vault() -> anyhow::Result<()> {
        if which::which("vault").is_err() {
            return Ok(());
        }
        let vault = VaultServer::spawn().await?;

        vault
            .put_from_file("example.com", "../../example-private-dkim-key.pem")
            .await?;

        let source = vault.make_source("example.com");
        let data = source.get().await?;

        assert_eq!(
            data,
            std::fs::read("../../example-private-dkim-key.pem").unwrap()
        );

        vault.put("foo", "bar").await?;

        let source = vault.make_source("foo");
        let data = source.get().await?;

        assert_eq!(data, b"bar",);

        Ok(())
    }
}
