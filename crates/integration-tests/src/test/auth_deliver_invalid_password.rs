use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

#[tokio::test]
async fn auth_deliver_invalid_password() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_SMTP_AUTH_USERNAME", "daniel"),
        ("KUMOD_SMTP_AUTH_PASSWORD", "needle"),
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
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await?;
    println!("Stopped!");

    let payload = data_encoding::BASE64.encode(b"\0daniel\0needle");
    let source_records = daemon.source.collect_logs().await?;
    let sink_records = daemon.sink.collect_logs().await?;
    for records in [&source_records, &sink_records] {
        let json = serde_json::to_string(records)?;
        anyhow::ensure!(!json.contains("needle"), "password in disposition logs");
        anyhow::ensure!(!json.contains(&payload), "AUTH payload in disposition logs");
    }
    let failure = source_records
        .iter()
        .find(|r| r.kind == TransientFailure)
        .expect("outbound AUTH failure");
    anyhow::ensure!(failure.response.content.contains("AUTH PLAIN [REDACTED]"));
    let rejection = sink_records
        .iter()
        .find(|r| r.kind == kumo_log_types::RecordType::Rejection)
        .expect("inbound AUTH rejection");
    anyhow::ensure!(rejection.response.code == 535);
    anyhow::ensure!(rejection.response.command.as_deref() == Some("AUTH PLAIN [REDACTED]"));

    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        TransientFailure: 1,
    },
    sink_counts: {
        Rejection: 1,
    },
}
"
    );
    Ok(())
}
