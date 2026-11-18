use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use bstr::ByteSlice;
use k9::assert_equal;
use kumo_log_types::RecordType;
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

    let client = kumo_http_client::builder().build().unwrap();
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

    daemon.assert_no_acct_deny().await?;
    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;

    // Verify the message content was properly expanded
    let body = parsed.body().unwrap();
    match body {
        mailparsing::DecodedBody::Text(text) => {
            assert!(text.as_bytes().contains_str("Hello Test User!"));
            assert!(text.as_bytes().contains_str("This is a compressed message"));
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
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json_data)?;
    let compressed_data = encoder.finish()?;

    let client = kumo_http_client::builder().build().unwrap();
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

#[tokio::test]
async fn http_inject_compression_zstd() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let expected_body = "Hello Test User! This is a Zstd-compressed message.\r\n".repeat(100);
    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{"email": "user@example.com"}],
        "template_dialect": "Static",
        "content": format!("Subject: Zstd Test\r\n\r\n{expected_body}")
    });
    let json_data = serde_json::to_vec(&payload)?;
    let compressed_data = zstd::stream::encode_all(json_data.as_slice(), 1)?;
    assert!(compressed_data.len() < json_data.len());

    let response = kumo_http_client::builder()
        .build()?
        .post(&format!(
            "http://{}/api/inject/v1",
            daemon.source.listener("http")
        ))
        .header("Content-Encoding", "zstd")
        .header("Content-Type", "application/json")
        .body(compressed_data)
        .send()
        .await?;

    assert_equal!(response.status(), 200);
    let response_json: serde_json::Value = response.json().await?;
    assert_equal!(response_json["success_count"], 1);
    assert_equal!(response_json["fail_count"], 0);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;
    daemon.stop_both().await.context("stop_both")?;
    daemon.assert_no_acct_deny().await?;

    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;
    assert_equal!(parsed.headers().subject().unwrap().unwrap(), "Zstd Test");
    match parsed.body().unwrap() {
        mailparsing::DecodedBody::Text(text) => {
            assert_equal!(text.as_bytes(), expected_body.as_bytes());
        }
        _ => panic!("Expected text body"),
    }

    Ok(())
}

#[tokio::test]
async fn http_inject_compression_zstd_invalid() -> anyhow::Result<()> {
    use std::io::Write;

    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{"email": "user@example.com"}],
        "content": "Subject: Invalid Zstd\r\n\r\nMust not be delivered"
    });
    let json_data = serde_json::to_vec(&payload)?;
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 1)?;
    encoder.include_checksum(true)?;
    encoder.write_all(&json_data)?;
    let compressed_data = encoder.finish()?;

    // Keep the decoded JSON intact, but omit part of the frame checksum.
    let truncated = compressed_data[..compressed_data.len() - 1].to_vec();
    let client = kumo_http_client::builder().build()?;
    for body in [b"not a zstd frame".to_vec(), truncated] {
        let response = client
            .post(&format!(
                "http://{}/api/inject/v1",
                daemon.source.listener("http")
            ))
            .header("Content-Encoding", "zstd")
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await?;
        assert_equal!(response.status(), 400);
    }

    daemon.stop_both().await.context("stop_both")?;
    assert_equal!(daemon.extract_maildir_messages()?.len(), 0);
    let summary = daemon.dump_logs().await.context("dump_logs")?;
    assert_equal!(summary.source_counts.get(&RecordType::Reception), None);

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

    let client = kumo_http_client::builder().build().unwrap();
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

    let client = kumo_http_client::builder().build().unwrap();
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

    assert_equal!(response.status(), 415);
    let supported = response.headers()["Accept-Encoding"].to_str()?;
    assert!(supported
        .split(',')
        .any(|encoding| encoding.trim() == "zstd"));

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}

#[tokio::test]
async fn http_response_compression() -> anyhow::Result<()> {
    use std::io::Read;

    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;
    let client = kumo_http_client::builder()
        .no_gzip()
        .no_deflate()
        .no_brotli()
        .no_zstd()
        .timeout(Duration::from_secs(10))
        .build()?;
    let endpoint = format!(
        "http://{}/api-docs/openapi.json",
        daemon.source.listener("http")
    );
    let baseline = client.get(&endpoint).send().await?;
    assert_equal!(baseline.status(), 200);
    assert!(baseline.headers().get("Content-Encoding").is_none());
    let expected = baseline.bytes().await?;
    assert!(expected.len() > 32);

    for (accept, encoding) in [
        ("identity", None),
        ("gzip", Some("gzip")),
        ("deflate", Some("deflate")),
        ("zstd", Some("zstd")),
        ("zstd;q=0", None),
        ("zstd;q=0, gzip", Some("gzip")),
        ("zstd, gzip;q=0.5", Some("zstd")),
        ("gzip, zstd;q=0.5", Some("gzip")),
    ] {
        let response = client
            .get(&endpoint)
            .header("Accept-Encoding", accept)
            .send()
            .await?;
        assert_equal!(response.status(), 200);
        assert_equal!(
            response
                .headers()
                .get("Content-Encoding")
                .map(|h| h.to_str().unwrap()),
            encoding
        );
        let body = response.bytes().await?;
        let mut decoded = Vec::new();
        match encoding {
            Some("gzip") => {
                flate2::read::GzDecoder::new(body.as_ref()).read_to_end(&mut decoded)?;
            }
            Some("deflate") => {
                flate2::read::ZlibDecoder::new(body.as_ref()).read_to_end(&mut decoded)?;
            }
            Some("zstd") => decoded = zstd::stream::decode_all(body.as_ref())?,
            None => decoded.extend_from_slice(&body),
            _ => unreachable!(),
        }
        assert_equal!(decoded.as_slice(), expected.as_ref());
    }

    daemon.stop_both().await.context("stop_both")?;
    daemon.assert_no_acct_deny().await?;
    Ok(())
}
