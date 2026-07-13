#![cfg(test)]
use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use futures_lite::stream::StreamExt;
use kumo_log_types::RecordType::Delivery;
use lapin::options::{BasicAckOptions, BasicConsumeOptions, QueueDeclareOptions};
use lapin::types::FieldTable;
use lapin::{Connection, ConnectionProperties};
use std::time::Duration;
use testcontainers_modules::rabbitmq::RabbitMq;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

#[tokio::test]
async fn test_lapin_rabbit() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

    let rabbitmq_instance = RabbitMq::default().start().await?;

    let amqp_url = format!(
        "amqp://{}:{}",
        rabbitmq_instance.get_host().await?,
        rabbitmq_instance.get_host_port_ipv4(5672).await?
    );

    eprintln!("made rabbit {amqp_url}");

    let options = ConnectionProperties::default()
        .with_executor(tokio_executor_trait::Tokio::current())
        .with_reactor(tokio_reactor_trait::Tokio);

    let connection = Connection::connect(&amqp_url, options).await?;
    let channel = connection.create_channel().await?;
    let queue = channel
        .queue_declare(
            "woot",
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .await?;
    println!("made queue {queue:?}");

    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_AMQPHOOK_URL", &amqp_url)]).await?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    // We get one reception which will be relayed to the sink.
    // The reception and the delivery will generate 2 amqp log records
    // which show as 2 additional deliveries, so 3 total deliveries
    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 3,
            Duration::from_secs(10),
        )
        .await;
    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 3,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );

    let mut consumer = channel
        .basic_consume(
            "woot",
            "my_consumer",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;

    let timeout = tokio::time::Duration::from_secs(20);

    // Wait for Reception record
    let reception = tokio::time::timeout(timeout, consumer.next())
        .await?
        .unwrap();
    let reception = reception?;
    println!("reception={}", String::from_utf8_lossy(&reception.data));
    reception.ack(BasicAckOptions::default()).await?;

    // Wait for Delivery record
    let delivery = tokio::time::timeout(timeout, consumer.next())
        .await?
        .unwrap();
    let delivery = delivery?;
    println!("delivery={}", String::from_utf8_lossy(&delivery.data));
    delivery.ack(BasicAckOptions::default()).await?;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    Ok(())
}

#[tokio::test]
async fn test_amqprs_rabbit() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

    let rabbitmq_instance = RabbitMq::default().start().await?;

    let amqp_host_port = format!(
        "{}:{}",
        rabbitmq_instance.get_host().await?,
        rabbitmq_instance.get_host_port_ipv4(5672).await?
    );

    let amqp_url = format!(
        "amqp://{}:{}",
        rabbitmq_instance.get_host().await?,
        rabbitmq_instance.get_host_port_ipv4(5672).await?
    );

    eprintln!("made rabbit {amqp_url}");

    let options = ConnectionProperties::default()
        .with_executor(tokio_executor_trait::Tokio::current())
        .with_reactor(tokio_reactor_trait::Tokio);

    let connection = Connection::connect(&amqp_url, options).await?;
    let channel = connection.create_channel().await?;
    let queue = channel
        .queue_declare(
            "woot",
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .await?;
    println!("made queue {queue:?}");

    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_AMQP_HOST_PORT", &amqp_host_port)]).await?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    // We get one reception which will be relayed to the sink.
    // The reception and the delivery will generate 2 amqp log records
    // which show as 2 additional deliveries, so 3 total deliveries
    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 3,
            Duration::from_secs(10),
        )
        .await;
    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 3,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let mut consumer = channel
        .basic_consume(
            "woot",
            "my_consumer",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;

    let timeout = tokio::time::Duration::from_secs(20);

    // Wait for Reception record
    let delivery = tokio::time::timeout(timeout, consumer.next())
        .await?
        .unwrap();
    let delivery = delivery?;
    println!("{}", String::from_utf8_lossy(&delivery.data));
    delivery.ack(BasicAckOptions::default()).await?;

    // Wait for Delivery record
    let delivery = tokio::time::timeout(timeout, consumer.next())
        .await?
        .unwrap();
    let delivery = delivery?;
    println!("{}", String::from_utf8_lossy(&delivery.data));
    delivery.ack(BasicAckOptions::default()).await?;

    Ok(())
}
