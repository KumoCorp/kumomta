use crate::kumod::{DaemonWithMaildir, MailGenParams};
use k9::assert_equal;
use kumo_log_types::RecordType::{Delivery, Reception};
use std::time::Duration;

/// Validate that we record and log TLS parameters for
/// both Reception and Delivery records
#[tokio::test]
async fn tls_info_log() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await?;
    println!("Stopped!");

    let source_logs = daemon.source.collect_logs().await?;

    let reception = source_logs
        .iter()
        .find(|record| record.kind == Reception)
        .unwrap();
    eprintln!("source: {reception:#?}");
    assert!(reception.tls_cipher.is_none());

    let delivery = source_logs
        .iter()
        .find(|record| record.kind == Delivery)
        .unwrap();
    eprintln!("source: {delivery:#?}");
    assert!(delivery.tls_cipher.is_some());

    let sink_logs = daemon.sink.collect_logs().await?;
    let reception = sink_logs
        .iter()
        .find(|record| record.kind == Reception)
        .unwrap();
    eprintln!("sink: {reception:#?}");
    assert_equal!(delivery.tls_cipher, reception.tls_cipher);
    assert_equal!(
        delivery.tls_protocol_version,
        reception.tls_protocol_version
    );

    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;
    let trace = parsed
        .headers()
        .get_first("Received")
        .unwrap()
        .as_unstructured()
        .unwrap();
    println!("trace: {trace}");
    assert!(trace.contains(&format!(
        "with ESMTPS ({}:{})",
        reception.tls_protocol_version.as_ref().unwrap(),
        reception.tls_cipher.as_ref().unwrap()
    )));

    Ok(())
}
