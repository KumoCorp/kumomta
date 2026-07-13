use crate::kumod::{generate_message_text, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use async_nats::jetstream::stream::Config;
use async_nats::jetstream::{self, consumer};
use async_nats::ConnectOptions;
use futures_lite::StreamExt;
use kumo_log_types::RecordType::Delivery;
use serde_json::json;
use testcontainers_modules::nats::Nats;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use tokio::time::Duration;

const SUBJECT: &str = "events";
const USERNAME: &str = "user";
const PASSWORD: &str = "userpassword";
const PORT: u16 = 4222;

fn get_nats_config() -> String {
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

    return nats_config;
}

#[tokio::test]
async fn test_nats() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

    let nats_config = get_nats_config();

    let nats_instance = Nats::default()
        .with_copy_to("/nats-server.conf", nats_config.as_bytes().to_vec())
        .with_container_name("nats")
        .start()
        .await?;

    let address = format!(
        "{}:{}",
        nats_instance.get_host().await?,
        nats_instance.get_host_port_ipv4(PORT).await?
    );

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

    let mut daemon = DaemonWithMaildirOptions::new()
        .env("NATS_ADDRESS", address)
        .env("NATS_SUBJECT", SUBJECT)
        .env("NATS_USERNAME", USERNAME)
        .env("NATS_PASSWORD", PASSWORD)
        .policy_file("source-nats.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let num_msgs = 3;
    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(1024, 78);
    for _ in 0..num_msgs {
        let response = MailGenParams {
            body: Some(&body),
            recip: Some("rec@nats"),
            ..Default::default()
        }
        .send(&mut client)
        .await
        .context("send message")?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);
    }

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 3,
            Duration::from_secs(10),
        )
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 3,
        Delivery: 3,
    },
    sink_counts: {},
}
"
    );

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

    assert_eq!(messages.count().await, num_msgs);

    Ok(())
}
