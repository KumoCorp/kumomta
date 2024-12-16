use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType;
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

#[tokio::test]
async fn tls_opportunistic_reconnect() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        // The default is OpportunisticInsecure; tighten it up a bit
        // so that we fail to deliver to the sink, and verify
        // the result of that attempt below
        ("KUMOD_ENABLE_TLS", "Opportunistic"),
        ("KUMOD_OPPORTUNISTIC_TLS_RECONNECT", "true"),
    ])
    .await?;

    let mut client = daemon.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(5),
        )
        .await;

    daemon.stop_both().await?;
    println!("Stopped!");

    let records = daemon.source.collect_logs()?;
    println!("{records:#?}");
    let delivery = records
        .iter()
        .find(|r| r.kind == RecordType::Delivery)
        .expect("to have a Delivery record");
    assert_eq!(delivery.tls_cipher, None, "tls should not have been used");
    assert_eq!(
        delivery.tls_protocol_version, None,
        "tls should not have been used"
    );
    assert_eq!(
        delivery.tls_peer_subject_name, None,
        "tls should not have been used"
    );

    let delivery_summary = daemon.dump_logs()?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );
    Ok(())
}
