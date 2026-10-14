use crate::kumod::DaemonWithMaildirOptions;
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

/// With the injection concurrency limit set to 1, a request that is held in
/// flight (the policy sleeps in http_message_generated) occupies the only slot.
/// A second request is rejected with a 503 when it arrives while that slot is
/// held and there aren't any free permits. The first request still completes
/// once it is no longer competing for the slot.
#[tokio::test]
async fn http_inject_concurrency_limit() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-httpinject-concurrency.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let url = format!("http://{}/api/inject/v1", daemon.source.listener("http"));
    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{ "email": "user@example.com" }],
        "content": { "subject": "concurrency", "text_body": "Hello!" },
    });

    let client = reqwest::Client::new();

    // First request holds the slot for the duration of the policy sleep.
    let first = {
        let client = client.clone();
        let url = url.clone();
        let payload = payload.clone();
        tokio::spawn(async move { client.post(&url).json(&payload).send().await })
    };

    // httpinject_requests_in_flight is incremented exactly when a request holds
    // a concurrency permit. Waiting for it to reach 1 confirms the first request
    // holds the only slot before the second request is sent.
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| m.name().as_str() == "httpinject_requests_in_flight",
            |values| values.iter().any(|v| *v >= 1.0),
        )
        .await
        .context("waiting for the first request to occupy the slot")?;

    let second = client.post(&url).json(&payload).send().await?;
    assert_equal!(
        second.status().as_u16(),
        503,
        "second overlapping request is rejected at the limit"
    );
    let second_body = second.text().await?;
    assert_equal!(
        &*second_body,
        "Error: too many concurrent injection requests"
    );

    let first = first.await.context("join first request")??;
    assert_equal!(
        first.status().as_u16(),
        200,
        "first request completes successfully"
    );

    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| m.name().as_str() == "httpinject_requests_rejected_total",
            |values| values.iter().any(|v| *v >= 1.0),
        )
        .await
        .context("rejection counter records the 503")?;

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}
