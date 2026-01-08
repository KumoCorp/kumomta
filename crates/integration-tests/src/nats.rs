#![cfg(test)]
use crate::kumod::DaemonWithMaildirOptions;
use anyhow::Context;
use async_nats::jetstream::stream::Config;
use async_nats::jetstream::{self, consumer};
use async_nats::ConnectOptions;
use futures_lite::StreamExt;
use serde_json::json;
use testcontainers_modules::nats::Nats;
use testcontainers_modules::testcontainers::core::ContainerPort;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

const SUBJECT: &str = "events";
const USERNAME: &str = "user";
const PASSWORD: &str = "userpassword";
const PORT: u16 = 4222;

fn get_nats_config() -> (String, String) {
    let address = format!("localhost:{PORT}");

    let nats_config = json!(
      {
        "host": "0.0.0.0",
        "port": PORT,
        "server_name": "nats",
        "client_advertise": address,
        "jetstream": {
          "store_dir": "/data/jetstream",
          "max_file": "10M",
        },
        "accounts": {
          "SYS": {
            "users": [{
              "user": "admin",
              "password": "adminpassword"
            }]
          },
          "users": {
            "users": [{
              "user": USERNAME,
              "pass": PASSWORD
            }],
            "jetstream": "enabled"
          }
        }
      }
    )
    .to_string();

    return (address, nats_config);
}

#[tokio::test]
async fn test_nats() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

    let (address, nats_config) = get_nats_config();

    let _nats_instance = Nats::default()
        .with_copy_to("/nats-server.conf", nats_config.as_bytes().to_vec())
        .with_container_name("nats")
        .with_mapped_port(PORT, ContainerPort::Tcp(PORT))
        .start()
        .await?;
    let client = ConnectOptions::new()
        .user_and_password(USERNAME.to_string(), PASSWORD.to_string())
        .connect(address.clone())
        .await?;
    let stream = jetstream::new(client)
        .create_stream(Config {
            name: SUBJECT.to_string(),
            max_messages: 100,
            ..Default::default()
        })
        .await?;

    let _ = DaemonWithMaildirOptions::new()
        .env("ADDRESS", address)
        .env("SUBJECT", SUBJECT)
        .env("USERNAME", USERNAME)
        .env("PASSWORD", PASSWORD)
        .policy_file("nats.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?
        .stop_both()
        .await?;

    let messages = stream
        .get_or_create_consumer(
            "pull",
            consumer::pull::Config {
                ..Default::default()
            },
        )
        .await?
        .fetch()
        .messages()
        .await?;

    assert_eq!(messages.count().await, 3);

    Ok(())
}
