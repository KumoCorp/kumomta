use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

/// Test HTTP injection with gzip compressed request body
#[tokio::test]
async fn http_inject_deferred() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [
            {
                "email": "user@example.com",
                "name": "Test User"
            }
        ],
        "deferred_generation": true,
        "content": {
            "text_body": "Hello {{ name }}! This is a message.",
            "subject": "Deferred Generation Test"
        }
    });

    let json_data = serde_json::to_vec(&payload)?;

    let client = reqwest::Client::new();
    let response = client
        .post(&format!(
            "http://{}/api/inject/v1",
            daemon.source.listener("http")
        ))
        .header("Content-Type", "application/json")
        .body(json_data)
        .send()
        .await?;

    anyhow::ensure!(
        response.status() == 200,
        "Response status: {}",
        response.status()
    );
    let response_json: serde_json::Value = response.json().await?;
    eprintln!("response: {response_json:?}");
    assert_equal!(
        response_json["success_count"],
        0,
        "deferred always shows zero"
    );
    assert_equal!(response_json["fail_count"], 0);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 2,
        Delivery: 2,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );

    daemon.assert_no_acct_deny().await?;
    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;

    // Verify the message content was properly expanded
    let body = parsed.body().unwrap();
    match body {
        mailparsing::DecodedBody::Text(text) => {
            assert!(text.contains("Hello Test User!"));
            assert!(text.contains("This is a message"));
        }
        _ => panic!("Expected text body"),
    }

    Ok(())
}
