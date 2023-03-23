use anyhow::anyhow;
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
                    key: Vec<u8>,
                }

                let entry: Entry = vaultrs::kv2::read(&client, vault_mount, vault_path).await?;

                Ok(entry.key)
            }
        }
    }
}
