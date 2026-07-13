use crate::kumod::{DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use k9::assert_equal;

/// Asserts that request_body_limit is effective and has a
/// known response
#[tokio::test]
async fn http_inject_size_limit() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let content = MailGenParams {
        // Ask for 2MB of mail body. The overall request
        // size will therefore be > 2MB, which is the
        // default value of `request_body_limit` set
        // in the http listener.
        size: Some(2 * 1024 * 1024),
        ..Default::default()
    }
    .generate()?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [
            {
                "email": "user@example.com",
                "name": "Test User"
            }
        ],
        "content": content,
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
        .bytes()
        .await
        .context("failed to read error response body")?;

    assert_equal!(status, 413, "Should be too large");
    assert_equal!(
        &*body_bytes,
        b"Failed to buffer the request body: length limit exceeded"
    );

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}

/// Asserts that request_body_limit is effective and has a
/// known response when using compressed requests
#[tokio::test]
async fn http_inject_size_limit_compressed() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let content = MailGenParams {
        // Ask for 2MB of mail body. The overall request
        // size will therefore be > 2MB, which is the
        // default value of `request_body_limit` set
        // in the http listener.
        size: Some(2 * 1024 * 1024),
        ..Default::default()
    }
    .generate()?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [
            {
                "email": "user@example.com",
                "name": "Test User"
            }
        ],
        "content": content,
    });

    let json_data = serde_json::to_vec(&payload)?;

    // Compress with gzip
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json_data)?;
    let compressed_data = encoder.finish()?;

    eprintln!(
        "Original size: {}, Compressed size: {}",
        json_data.len(),
        compressed_data.len()
    );

    let client = reqwest::Client::new();
    let response = client
        .post(&format!(
            "http://{}/api/inject/v1",
            daemon.source.listener("http")
        ))
        .header("Content-Encoding", "gzip")
        .header("Content-Type", "application/json")
        .body(compressed_data)
        .send()
        .await?;

    let status = response.status();
    let body_bytes = response
        .bytes()
        .await
        .context("failed to read error response body")?;

    // We still expect the 2MB limit to apply, even though compression
    // means that we sent only about 600KB on the wire.
    assert_equal!(status, 413, "Should be too large");
    assert_equal!(
        &*body_bytes,
        b"Failed to buffer the request body: length limit exceeded"
    );

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}
