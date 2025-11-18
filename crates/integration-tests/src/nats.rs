#![cfg(test)]
use crate::kumod::DaemonWithMaildirOptions;
use anyhow::Context;
use async_nats::jetstream::stream::Config;
use async_nats::jetstream::{self, consumer};
use async_nats::ConnectOptions;
use futures_lite::StreamExt;
use lazy_static::lazy_static;
use serde_json::{json, Value};
use testcontainers_modules::nats::Nats;
use testcontainers_modules::testcontainers::core::ContainerPort;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

const SUBJECT: &str = "events";
const USERNAME: &str = "user";
const PASSWORD: &str = "userpassword";
const PORT: u16 = 4222;

lazy_static! {
    static ref ADDR: String = format!("localhost:{PORT}");
    static ref NATS_CONFIG: Value = json!(
      {
        "host": "0.0.0.0",
        "port": PORT,
        "server_name": "nats",
        "client_advertise": ADDR.to_string(),
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
    );
}

#[tokio::test]
async fn test_nats() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

    let _nats_instance = Nats::default()
        .with_copy_to(
            "/nats-server.conf",
            NATS_CONFIG.to_string().as_bytes().to_vec(),
        )
        .with_container_name("nats")
        .with_mapped_port(PORT, ContainerPort::Tcp(PORT))
        .start()
        .await?;
    let client = ConnectOptions::new()
        .user_and_password(USERNAME.to_string(), PASSWORD.to_string())
        .connect(ADDR.to_string())
        .await?;
    let stream = jetstream::new(client)
        .create_stream(Config {
            name: SUBJECT.to_string(),
            max_messages: 100,
            ..Default::default()
        })
        .await?;

    let _ = DaemonWithMaildirOptions::new()
        .env("ADDRESS", ADDR.to_string())
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
