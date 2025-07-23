use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

/// Test HTTP injection with gzip compressed request body
#[tokio::test]
async fn http_inject_compression_gzip() -> anyhow::Result<()> {
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
            "text_body": "Hello {{ name }}! This is a compressed message. ".repeat(100),
            "subject": "Compression Test"
        }
    });

    let json_data = serde_json::to_vec(&payload)?;

    // Compress with gzip
    use flate2::{write::GzEncoder, Compression};
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

    anyhow::ensure!(
        response.status() == 200,
        "Response status: {}",
        response.status()
    );
    let response_json: serde_json::Value = response.json().await?;
    assert_equal!(response_json["success_count"], 1);
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

    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;

    // Verify the message content was properly expanded
    let body = parsed.body().unwrap();
    match body {
        mailparsing::DecodedBody::Text(text) => {
            assert!(text.contains("Hello Test User!"));
            assert!(text.contains("This is a compressed message"));
        }
        _ => panic!("Expected text body"),
    }

    Ok(())
}

/// Test HTTP injection with deflate compressed request body
#[tokio::test]
async fn http_inject_compression_deflate() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{"email": "user@example.com"}],
        "content": "Subject: Deflate Test\r\n\r\nThis message was compressed with deflate"
    });

    let json_data = serde_json::to_vec(&payload)?;

    // Compress with deflate (zlib format)
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json_data)?;
    let compressed_data = encoder.finish()?;

    let client = reqwest::Client::new();
    let response = client
        .post(&format!(
            "http://{}/api/inject/v1",
            daemon.source.listener("http")
        ))
        .header("Content-Encoding", "deflate")
        .header("Content-Type", "application/json")
        .body(compressed_data)
        .send()
        .await?;

    anyhow::ensure!(response.status() == 200);
    let response_json: serde_json::Value = response.json().await?;
    assert_equal!(response_json["success_count"], 1);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;

    assert_equal!(parsed.headers().subject().unwrap().unwrap(), "Deflate Test");

    Ok(())
}

/// Test that uncompressed requests still work after compression feature is added
#[tokio::test]
async fn http_inject_uncompressed_backward_compatibility() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{"email": "user@example.com"}],
        "content": "Subject: Backward Compatibility\r\n\r\nThis message is not compressed"
    });

    let client = reqwest::Client::new();
    let response = client
        .post(&format!(
            "http://{}/api/inject/v1",
            daemon.source.listener("http")
        ))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await?;

    anyhow::ensure!(response.status() == 200);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);

    Ok(())
}

/// Test error handling for unsupported compression encoding
#[tokio::test]
async fn http_inject_unsupported_compression() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{"email": "user@example.com"}],
        "content": "Subject: Test\r\n\r\nTest message"
    });

    let client = reqwest::Client::new();
    let response = client
        .post(&format!(
            "http://{}/api/inject/v1",
            daemon.source.listener("http")
        ))
        .header("Content-Encoding", "brotli") // Unsupported
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await?;

    // Should return error for unsupported encoding
    anyhow::ensure!(
        response.status() != 200,
        "Expected error for unsupported encoding"
    );

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}
