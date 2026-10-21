use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

#[tokio::test]
async fn auth_deliver_invalid_password() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_SMTP_AUTH_USERNAME", "daniel"),
        ("KUMOD_SMTP_AUTH_PASSWORD", "incorrect-password"),
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

    // credential_b64 is the base64 of "\0daniel\0incorrect-password", the
    // AUTH PLAIN payload the source sends inline. The sink's rejection of that
    // payload produces both records under test: a TransientFailure logged by
    // the source and a Rejection logged by the sink. Check both the base64
    // payload and the plaintext password, to catch a path that logs the
    // password undecoded as well as one that logs the encoded payload.
    let credential_b64 = "AGRhbmllbABpbmNvcnJlY3QtcGFzc3dvcmQ=";
    let mut saw_redacted_command = false;
    for daemon in [&daemon.source, &daemon.sink] {
        for record in daemon.collect_logs().await? {
            let json = serde_json::to_string(&record)?;
            anyhow::ensure!(
                !json.contains(credential_b64) && !json.contains("incorrect-password"),
                "credential leaked into a log record: {json}"
            );
            if record.response.command.as_deref() == Some("AUTH PLAIN [REDACTED]") {
                saw_redacted_command = true;
            }
        }
    }
    anyhow::ensure!(
        saw_redacted_command,
        "expected a record whose command was the redacted AUTH PLAIN line"
    );

    daemon.stop_both().await?;
    println!("Stopped!");

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
