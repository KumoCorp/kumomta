use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use bstr::ByteSlice;
use std::time::Duration;

/// Confirm that independently templated injection entries keep their own
/// dialects and substitutions when sent in a single HTTP batch.
#[tokio::test]
async fn http_inject_batch_templates_remain_independent() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;

    let payload = serde_json::json!({
        "requests": [
            {
                "envelope_sender": "sender@example.com",
                "template_dialect": "Jinja",
                "content": {
                    "subject": "jinja",
                    "text_body": "Hi {{ name }}, campaign {{ campaign }}."
                },
                "substitutions": {"campaign": "autumn"},
                "recipients": [{"email": "alice@example.com", "name": "Alice"}]
            },
            {
                "envelope_sender": "sender@example.com",
                "template_dialect": "Handlebars",
                "content": {
                    "subject": "handlebars",
                    "text_body": "Hi {{ name }}, campaign {{ campaign }}."
                },
                "substitutions": {"campaign": "wrong-global"},
                "recipients": [{
                    "email": "bob@example.com",
                    "name": "Bob",
                    "substitutions": {"campaign": "winter"}
                }]
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
    let body: serde_json::Value = response.json().await?;
    let outcomes = body["results"].as_array().context("results array")?;
    assert_eq!(outcomes.len(), 2);
    for (index, outcome) in outcomes.iter().enumerate() {
        assert_eq!(outcome["index"].as_u64(), Some(index as u64));
        assert_eq!(outcome["status"], 200);
        assert_eq!(outcome["response"]["success_count"], 1);
        assert_eq!(outcome["response"]["fail_count"], 0);
    }

    daemon
        .wait_for_maildir_count(2, Duration::from_secs(10))
        .await;
    daemon.stop_both().await?;

    let mut messages = daemon.extract_maildir_messages()?;
    assert_eq!(messages.len(), 2);
    let mut rendered_bodies = Vec::new();
    for message in &mut messages {
        let parsed = message.parsed()?;
        match parsed.body().unwrap() {
            mailparsing::DecodedBody::Text(text) => {
                rendered_bodies.push(String::from_utf8_lossy(text.as_bytes()).into_owned());
            }
            _ => panic!("expected a rendered text body"),
        }
    }

    assert!(rendered_bodies
        .iter()
        .any(|body| body.contains("Hi Alice, campaign autumn.")));
    assert!(rendered_bodies
        .iter()
        .any(|body| body.contains("Hi Bob, campaign winter.")));
    assert!(!rendered_bodies
        .iter()
        .any(|body| body.contains("wrong-global")));
    Ok(())
}
