use crate::kumod::KumoDaemon;
use data_encoding::BASE64;
use kumo_log_types::RecordType::Rejection;
use rfc5321::{Command, TlsOptions};

const REDACTED: &str = "AUTH PLAIN [REDACTED]";

fn auth_payload(user: &str) -> String {
    BASE64.encode(format!("\0{user}\0needle").as_bytes())
}

async fn check_rejection(
    command: &str,
    continuation: Option<&str>,
    expected_code: u16,
    expected_command: &str,
) -> anyhow::Result<()> {
    let mut daemon = KumoDaemon::spawn_with_policy("auth-log-redaction.lua").await?;
    let mut client = daemon.smtp_client("localhost").await?;
    client
        .starttls(TlsOptions {
            insecure: true,
            ..Default::default()
        })
        .await?;
    client.ehlo("localhost").await?;

    // Send verbatim so malformed AUTH and bare continuation lines reach the server.
    let mut response = client
        .send_command(&Command::Unknown(command.into()))
        .await?;
    if let Some(continuation) = continuation {
        anyhow::ensure!(response.code == 334, "expected AUTH challenge");
        response = client
            .send_command(&Command::Unknown(continuation.into()))
            .await?;
    }
    anyhow::ensure!(response.code == expected_code, "{response:?}");
    let response = client.send_command(&Command::Noop(None)).await?;
    anyhow::ensure!(response.code == 250, "session should remain usable");
    drop(client);
    daemon.stop().await?;

    let records = daemon.collect_logs().await?;
    let rejections: Vec<_> = records.iter().filter(|r| r.kind == Rejection).collect();
    anyhow::ensure!(rejections.len() == 1, "{rejections:?}");
    let rejection = rejections[0];
    anyhow::ensure!(rejection.response.code == expected_code);
    anyhow::ensure!(rejection.response.command.as_deref() == Some(expected_command));
    anyhow::ensure!(
        rejection
            .meta
            .get("response_command")
            .and_then(|v| v.as_str())
            == Some(expected_command),
        "response-rewrite callback must receive sanitized AUTH context"
    );
    let json = serde_json::to_string(&records)?;
    anyhow::ensure!(!json.contains("needle"), "password in disposition logs");
    if let Some(payload) = continuation.or_else(|| command.split_whitespace().nth(2)) {
        anyhow::ensure!(!json.contains(payload), "AUTH payload in disposition logs");
    }
    Ok(())
}

#[tokio::test]
async fn auth_continuation_password_redacted() -> anyhow::Result<()> {
    check_rejection("AUTH PLAIN", Some(&auth_payload("daniel")), 535, REDACTED).await
}

#[tokio::test]
async fn auth_malformed_command_redacted() -> anyhow::Result<()> {
    check_rejection(
        &format!("AUTH PLAIN {} !", auth_payload("daniel")),
        None,
        501,
        "AUTH [REDACTED]",
    )
    .await
}

#[tokio::test]
async fn auth_callback_rejection_redacted() -> anyhow::Result<()> {
    check_rejection(
        &format!("AUTH PLAIN {}", auth_payload("defer")),
        None,
        454,
        REDACTED,
    )
    .await
}
