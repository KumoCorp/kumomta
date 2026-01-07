#![cfg(test)]
use crate::kumod::{generate_message_text, MailGenParams};
use anyhow::Context;
use std::time::Duration;
use testcontainers_modules::testcontainers::core::{ContainerPort, WaitFor};
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};

/// Helper struct to manage Rspamd container lifecycle
struct RspamdContainer {
    #[allow(dead_code)]
    container: ContainerAsync<GenericImage>,
    url: String,
}

impl RspamdContainer {
    /// Start Rspamd container and wait for it to be ready
    async fn new() -> anyhow::Result<Self> {
        // Start Rspamd container
        let rspamd_image = GenericImage::new("rspamd/rspamd", "latest")
            .with_exposed_port(ContainerPort::Tcp(11333))
            .with_wait_for(WaitFor::seconds(5));

        let rspamd_container = rspamd_image.start().await?;

        let rspamd_host = rspamd_container.get_host().await?;
        let rspamd_port = rspamd_container.get_host_port_ipv4(11333).await?;
        let rspamd_url = format!("http://{rspamd_host}:{rspamd_port}");

        eprintln!("Started Rspamd at {rspamd_url}");

        // Wait for Rspamd to fully initialize and verify it's responding
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Verify Rspamd is responding
        let client = reqwest::Client::new();
        for attempt in 1..=10 {
            match client.get(format!("{}/ping", rspamd_url)).send().await {
                Ok(resp) if resp.status().is_success() => {
                    eprintln!("Rspamd is ready after {} attempts", attempt);
                    break;
                }
                _ => {
                    if attempt == 10 {
                        anyhow::bail!("Rspamd did not become ready in time");
                    }
                    eprintln!("Waiting for Rspamd to be ready (attempt {})", attempt);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        Ok(Self {
            container: rspamd_container,
            url: rspamd_url,
        })
    }

    /// Get the Rspamd base URL
    fn url(&self) -> &str {
        &self.url
    }
}

/// Helper struct to hold parsed Rspamd headers from a delivered message
#[derive(Debug)]
struct RspamdHeaders {
    spam_flag: Option<String>,
    spam_score: Option<String>,
    spam_action: Option<String>,
}

impl RspamdHeaders {
    /// Extract Rspamd headers from a maildir entry
    fn from_entry(entry: &mut maildir::MailEntry) -> anyhow::Result<Self> {
        let headers = entry.headers()?;

        Ok(Self {
            spam_flag: headers
                .get_first("X-Spam-Flag")
                .and_then(|h| h.as_unstructured().ok()),
            spam_score: headers
                .get_first("X-Spam-Score")
                .and_then(|h| h.as_unstructured().ok()),
            spam_action: headers
                .get_first("X-Spam-Action")
                .and_then(|h| h.as_unstructured().ok()),
        })
    }

    /// Assert that X-Spam-Flag header is present
    fn assert_has_flag(&self) -> anyhow::Result<&Self> {
        anyhow::ensure!(
            self.spam_flag.is_some(),
            "Expected X-Spam-Flag header to be present"
        );
        Ok(self)
    }

    /// Assert that X-Spam-Flag header has a specific value
    fn assert_flag_equals(&self, expected: &str) -> anyhow::Result<&Self> {
        match &self.spam_flag {
            Some(flag) => {
                anyhow::ensure!(
                    flag == expected,
                    "Expected X-Spam-Flag to be '{}', got '{}'",
                    expected,
                    flag
                );
            }
            None => {
                anyhow::bail!("Expected X-Spam-Flag header to be present");
            }
        }
        Ok(self)
    }

    /// Assert that X-Spam-Score header is present and can be parsed as a float
    fn assert_has_score(&self) -> anyhow::Result<f64> {
        match &self.spam_score {
            Some(score) => {
                let parsed = score
                    .parse::<f64>()
                    .context(format!("Failed to parse X-Spam-Score '{}' as float", score))?;
                Ok(parsed)
            }
            None => {
                anyhow::bail!("Expected X-Spam-Score header to be present");
            }
        }
    }

    /// Assert that X-Spam-Action header is present
    fn assert_has_action(&self) -> anyhow::Result<&Self> {
        anyhow::ensure!(
            self.spam_action.is_some(),
            "Expected X-Spam-Action header to be present"
        );
        Ok(self)
    }

    /// Assert that X-Spam-Action header has a specific value
    fn assert_action_equals(&self, expected: &str) -> anyhow::Result<&Self> {
        match &self.spam_action {
            Some(action) => {
                anyhow::ensure!(
                    action == expected,
                    "Expected X-Spam-Action to be '{}', got '{}'",
                    expected,
                    action
                );
            }
            None => {
                anyhow::bail!("Expected X-Spam-Action header to be present");
            }
        }
        Ok(self)
    }
}

#[tokio::test]
async fn test_rspamd_scan_message() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_default() != "1" {
        return Ok(());
    }

    let rspamd = RspamdContainer::new().await?;

    let mut daemon = crate::kumod::DaemonWithMaildirOptions::new()
        .policy_file("rspamd.lua")
        .env("KUMOD_TEST_RSPAMD_URL", rspamd.url())
        .start()
        .await?;

    eprintln!("Sending test message to scan@test.example.com");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("scan@test.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;

    eprintln!("SMTP response: {response:?}");
    anyhow::ensure!(
        response.code == 250,
        "Expected 250 response, got {}",
        response.code
    );

    // Wait for message to be delivered to sink's maildir
    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    // Extract and verify Rspamd headers were added
    let mut messages = daemon.extract_maildir_messages()?;
    anyhow::ensure!(
        messages.len() == 1,
        "Expected 1 message, found {}",
        messages.len()
    );

    let rspamd_headers = RspamdHeaders::from_entry(&mut messages[0])?;
    eprintln!("Rspamd headers: {rspamd_headers:?}");

    // Verify headers are present and message was scanned
    rspamd_headers.assert_has_flag()?.assert_has_action()?;

    let score = rspamd_headers.assert_has_score()?;
    eprintln!("Spam score: {score}");

    eprintln!("Message successfully scanned and delivered with headers");

    daemon.stop_both().await.context("stop_both")?;
    eprintln!("Test completed successfully");

    Ok(())
}

/// Test that messages containing the GTUBE spam test pattern are rejected.
///
/// GTUBE (Generic Test for Unsolicited Bulk Email) is a standard test pattern
/// that spam filters recognize as spam. This test sends a message containing
/// the GTUBE pattern to reject-spam@test.example.com, which is configured to
/// reject messages when Rspamd returns certain spam actions.
#[tokio::test]
async fn test_rspamd_reject_spam() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_default() != "1" {
        return Ok(());
    }

    let rspamd = RspamdContainer::new().await?;

    let mut daemon = crate::kumod::DaemonWithMaildirOptions::new()
        .policy_file("rspamd.lua")
        .env("KUMOD_TEST_RSPAMD_URL", rspamd.url())
        .start()
        .await?;

    eprintln!("Sending GTUBE spam test message to reject-spam@test.example.com");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    // Send message with GTUBE spam test pattern
    // GTUBE pattern is recognized by Rspamd as spam
    let gtube_body = "This is a test message.\r\n\
        \r\n\
        XJS*C4JDBQADN1.NSBN3*2IDNEN*GTUBE-STANDARD-ANTI-UBE-TEST-EMAIL*C.34X\r\n\
        \r\n\
        If you received this message, your spam filter recognized the GTUBE pattern.\r\n";

    let response = MailGenParams {
        body: Some(gtube_body),
        recip: Some("reject-spam@test.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;

    eprintln!("SMTP response: {response:?}");

    // Message should be rejected due to spam content
    anyhow::ensure!(
        response.code >= 500 && response.code < 600,
        "Expected 5xx rejection code for spam message, got {}",
        response.code
    );

    daemon.stop_both().await.context("stop_both")?;
    eprintln!("Test completed successfully - spam was rejected");

    Ok(())
}

#[tokio::test]
async fn test_rspamd_headers() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_default() != "1" {
        return Ok(());
    }

    let rspamd = RspamdContainer::new().await?;

    let mut daemon = crate::kumod::DaemonWithMaildirOptions::new()
        .policy_file("rspamd.lua")
        .env("KUMOD_TEST_RSPAMD_URL", rspamd.url())
        .start()
        .await?;

    eprintln!("Sending test message to headers@test.example.com");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(512, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("headers@test.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;

    eprintln!("SMTP response: {response:?}");
    anyhow::ensure!(
        response.code == 250,
        "Expected 250 response, got {}",
        response.code
    );

    // Wait for message to be delivered to sink's maildir
    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    // Extract and verify Rspamd headers were added
    let mut messages = daemon.extract_maildir_messages()?;
    anyhow::ensure!(
        messages.len() == 1,
        "Expected 1 message, found {}",
        messages.len()
    );

    let rspamd_headers = RspamdHeaders::from_entry(&mut messages[0])?;
    eprintln!("Rspamd headers: {rspamd_headers:?}");

    // Verify headers are present and have correct values
    rspamd_headers
        .assert_has_flag()?
        .assert_flag_equals("NO")?
        .assert_has_action()?
        .assert_action_equals("no action")?;

    let score = rspamd_headers.assert_has_score()?;
    eprintln!("Spam score: {score}");

    // Normal messages should have a low score
    anyhow::ensure!(
        score < 5.0,
        "Expected spam score < 5.0 for normal message, got {score}"
    );

    daemon.stop_both().await.context("stop_both")?;
    eprintln!("Test completed successfully");

    Ok(())
}

#[tokio::test]
async fn test_rspamd_per_recipient_threshold() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_default() != "1" {
        return Ok(());
    }

    let rspamd = RspamdContainer::new().await?;

    let mut daemon = crate::kumod::DaemonWithMaildirOptions::new()
        .policy_file("rspamd.lua")
        .env("KUMOD_TEST_RSPAMD_URL", rspamd.url())
        .start()
        .await?;

    eprintln!("Sending test message to VIP recipient");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    // Send to VIP recipient (should be accepted even with any score)
    let body = generate_message_text(512, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("user@vip.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message to VIP")?;

    eprintln!("SMTP response for VIP: {response:?}");
    anyhow::ensure!(
        response.code == 250,
        "Expected 250 response for VIP recipient, got {}",
        response.code
    );

    // Wait for message to be delivered to source daemon's local maildir
    daemon
        .source
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    eprintln!("VIP message delivered successfully");

    daemon.stop_both().await.context("stop_both")?;
    eprintln!("Test completed successfully");

    Ok(())
}
