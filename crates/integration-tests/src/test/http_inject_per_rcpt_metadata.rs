use crate::kumod::DaemonWithMaildirOptions;
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::RecordType;
use std::time::Duration;

fn daemon_opts() -> DaemonWithMaildirOptions {
    DaemonWithMaildirOptions::new().policy_file("source-per-rcpt-metadata.lua")
}

/// Inject a message with per-recipient metadata and verify that the
/// metadata is stored under the `extra` meta key, accessible from Lua
/// via `msg:get_meta('extra')` and captured into log records via the
/// log's `meta` list.
#[tokio::test]
async fn per_recipient_metadata_captured_in_logs() -> anyhow::Result<()> {
    let mut daemon = daemon_opts()
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{
            "email": "user@example.com",
            "metadata": {
                "campaign": "promo-2026-q2",
                "segment": "premium"
            }
        }],
        "content": {
            "subject": "Rcpt Meta Log Test",
            "text_body": "Hello!"
        }
    });

    let body = daemon
        .source
        .api_client()
        .inject_v1(&payload)
        .await?;
    assert_equal!(body.success_count, 1);
    assert_equal!(body.fail_count, 0);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    // Stop before reading logs — kumod flushes log segments on shutdown.
    daemon.stop_both().await.context("stop_both")?;

    // Read source log records and find the Delivery entry.
    let logs = daemon.source.collect_logs().await?;
    let delivery = logs
        .iter()
        .find(|r| r.kind == RecordType::Delivery)
        .context("no Delivery log record found")?;

    let extra = delivery
        .meta
        .get("extra")
        .context("`extra` is missing from the Delivery log record")?;

    assert_equal!(
        extra.get("campaign").and_then(|v| v.as_str()),
        Some("promo-2026-q2")
    );
    assert_equal!(
        extra.get("segment").and_then(|v| v.as_str()),
        Some("premium")
    );

    Ok(())
}

/// Inject two recipients with distinct metadata and verify that each
/// Delivery log record carries only its own metadata under the default
/// `extra` key — metadata must not bleed across recipients.
#[tokio::test]
async fn per_recipient_metadata_multiple_recipients() -> anyhow::Result<()> {
    let mut daemon = daemon_opts()
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [
            {
                "email": "alice@example.com",
                "metadata": {
                    "campaign": "summer-sale",
                    "segment": "vip"
                }
            },
            {
                "email": "bob@example.com",
                "metadata": {
                    "campaign": "newsletter",
                    "segment": "standard"
                }
            }
        ],
        "content": {
            "subject": "Per-Recipient Meta Test",
            "text_body": "Hello!"
        }
    });

    let body = daemon
        .source
        .api_client()
        .inject_v1(&payload)
        .await?;
    assert_equal!(body.success_count, 2);
    assert_equal!(body.fail_count, 0);

    daemon
        .wait_for_maildir_count(2, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    let deliveries: Vec<_> = logs
        .iter()
        .filter(|r| r.kind == RecordType::Delivery)
        .collect();
    anyhow::ensure!(
        deliveries.len() == 2,
        "expected 2 Delivery records, got {}",
        deliveries.len()
    );

    // Match each delivery record to its recipient by address.
    let find_delivery = |addr: &str| {
        deliveries
            .iter()
            .find(|r| r.recipient.iter().any(|a| a.contains(addr)))
            .with_context(|| format!("no Delivery record for {addr}"))
    };

    let alice = find_delivery("alice")?;
    let alice_meta = alice
        .meta
        .get("extra")
        .context("`extra` missing from alice's Delivery record")?;
    assert_equal!(
        alice_meta.get("campaign").and_then(|v| v.as_str()),
        Some("summer-sale")
    );
    assert_equal!(
        alice_meta.get("segment").and_then(|v| v.as_str()),
        Some("vip")
    );

    let bob = find_delivery("bob")?;
    let bob_meta = bob
        .meta
        .get("extra")
        .context("`extra` missing from bob's Delivery record")?;
    assert_equal!(
        bob_meta.get("campaign").and_then(|v| v.as_str()),
        Some("newsletter")
    );
    assert_equal!(
        bob_meta.get("segment").and_then(|v| v.as_str()),
        Some("standard")
    );

    Ok(())
}

/// When no per-recipient metadata is supplied, `extra` must be absent
/// from the log record — the meta entry must not be created.
#[tokio::test]
async fn absent_metadata_is_not_in_logs() -> anyhow::Result<()> {
    let mut daemon = daemon_opts()
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{
            "email": "user@example.com"
        }],
        "content": {
            "subject": "No Metadata Test",
            "text_body": "Hello!"
        }
    });

    let body = daemon
        .source
        .api_client()
        .inject_v1(&payload)
        .await?;
    assert_equal!(body.success_count, 1);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    let delivery = logs
        .iter()
        .find(|r| r.kind == RecordType::Delivery)
        .context("no Delivery log record found")?;

    assert!(
        delivery.meta.get("extra").is_none(),
        "`extra` must be absent when no metadata was supplied"
    );

    Ok(())
}
