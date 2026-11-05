use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use bstr::ByteSlice;
use std::time::Duration;

/// Different message content is accepted in one HTTP call; a malformed
/// template fails only its own entry, and later entries still run.
#[tokio::test]
async fn http_inject_batch_mixed_results() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "requests": [
            {
                "envelope_sender": "sender@example.com",
                "content": {"subject": "batch one", "text_body": "Literal {{ name }} one"},
                "template_dialect": "Static",
                "recipients": [{"email": "one@example.com"}]
            },
            {
                "envelope_sender": "sender@example.com",
                "content": {"subject": "bad template", "text_body": "Hello {{ name"},
                "recipients": [{"email": "bad@example.com"}]
            },
            {
                "envelope_sender": "another@example.com",
                "content": {"subject": "batch three", "text_body": "Literal {{ name }} three"},
                "template_dialect": "Static",
                "recipients": [{"email": "three@example.com"}]
            }
        ]
    });

    let client = kumo_http_client::builder().build()?;
    let response = client
        .post(&format!(
            "http://{}/api/inject/batch/v1",
            daemon.source.listener("http")
        ))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status().as_u16(), 200);
    let result: serde_json::Value = response.json().await?;
    let results = result["results"].as_array().context("results array")?;
    assert_eq!(results.len(), 3);

    assert_eq!(results[0]["index"], 0);
    assert_eq!(results[0]["status"], 200);
    assert_eq!(results[0]["response"]["success_count"], 1);

    assert_eq!(results[1]["index"], 1);
    assert_eq!(results[1]["status"], 422);
    assert!(results[1]["error"].as_str().unwrap().contains("template"));

    assert_eq!(results[2]["index"], 2);
    assert_eq!(results[2]["status"], 200);
    assert_eq!(results[2]["response"]["success_count"], 1);

    daemon
        .wait_for_maildir_count(2, Duration::from_secs(10))
        .await;
    daemon.stop_both().await.context("stop_both")?;
    let mut messages = daemon.extract_maildir_messages()?;
    assert_eq!(messages.len(), 2);
    let mut bodies = Vec::new();
    for message in &mut messages {
        let parsed = message.parsed()?;
        match parsed.body().unwrap() {
            mailparsing::DecodedBody::Text(text) => {
                bodies.push(String::from_utf8_lossy(text.as_bytes()).into_owned());
            }
            _ => panic!("expected a text body"),
        }
    }
    assert!(bodies.iter().any(|body| body.contains("Literal {{ name }} one")));
    assert!(bodies.iter().any(|body| body.contains("Literal {{ name }} three")));
    Ok(())
}

#[tokio::test]
async fn http_inject_batch_rejects_empty_requests() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let client = kumo_http_client::builder().build()?;
    let response = client
        .post(&format!(
            "http://{}/api/inject/batch/v1",
            daemon.source.listener("http")
        ))
        .json(&serde_json::json!({"requests": []}))
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 400);
    daemon.stop_both().await?;
    Ok(())
}

#[tokio::test]
async fn http_inject_batch_invalid_address_rejects_all() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let client = kumo_http_client::builder().build()?;
    let payload = serde_json::json!({
        "requests": [
            {
                "envelope_sender": "sender@example.com",
                "content": {"subject": "valid first", "text_body": "Hello"},
                "recipients": [{"email": "valid@example.com"}]
            },
            {
                "envelope_sender": "sender@example.com",
                "content": {"subject": "invalid second", "text_body": "Hello"},
                "recipients": [{"email": "not-an-email"}]
            }
        ]
    });

    let response = client
        .post(&format!(
            "http://{}/api/inject/batch/v1",
            daemon.source.listener("http")
        ))
        .json(&payload)
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 422);
    let body = response.text().await?;
    assert!(body.contains("requests[1].recipients[0].email"));

    daemon.stop_both().await?;
    assert_eq!(daemon.extract_maildir_messages()?.len(), 0);
    Ok(())
}
