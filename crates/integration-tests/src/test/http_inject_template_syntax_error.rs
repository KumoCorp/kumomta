use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use k9::assert_equal;

/// Asserts that template syntax errors come back with a 422 status
#[tokio::test]
async fn http_inject_template_syntax_error() -> anyhow::Result<()> {
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
        "content": {
            "text_body": "Hello {{name}}, your code is {{code",
        },
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

    let status = response.status();
    let body_bytes = response
        .text()
        .await
        .context("failed to read error response body")?;

    assert_equal!(status, 422, "Should be unprocessable");
    assert_equal!(
        &*body_bytes,
        "Error: failed parsing field 'content.text_body' as template: \
        syntax error: unexpected end of input, expected end of variable block \
        (in template 'text_body.txt' line 1: 'Hello {{name}}, your code is {{code')"
    );

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}

#[tokio::test]
async fn http_inject_deferred_template_syntax_error() -> anyhow::Result<()> {
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
            "from": {
                "email": "bork bork",
            },
            "text_body": "The error is in the from email this time",
        },
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

    let status = response.status();
    let body_bytes = response
        .text()
        .await
        .context("failed to read error response body")?;

    assert_equal!(status, 422, "Should be unprocessable");
    assert_equal!(
        &*body_bytes,
        "Error: failed parsing content.from: invalid header: 0: at line 1:\n\
        bork bork\n     ^___\n\
        expected '@', found b\n\
        \n\
        1: at line 1, in addr_spec:\n\
        bork bork\n\
        ^________\n\n"
    );

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}
