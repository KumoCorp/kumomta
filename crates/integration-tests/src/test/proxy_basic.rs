use crate::kumod::{DaemonWithMaildir, DaemonWithMaildirOptions, MailGenParams};
use crate::kumoproxy::{ProxyArgs, ProxyDaemon};
use anyhow::Context;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

/// Helper struct for end-to-end proxy tests
/// Spawns: source kumod -> proxy -> sink kumod
struct DaemonWithProxy {
    kumod: DaemonWithMaildir,
    proxy: ProxyDaemon,
}

impl DaemonWithProxy {
    async fn spawn(
        proxy_args: ProxyArgs,
        kumod_env: Vec<(String, String)>,
    ) -> anyhow::Result<Self> {
        let proxy = ProxyDaemon::spawn(proxy_args)
            .await
            .context("ProxyDaemon::spawn")?;

        let proxy_addr = proxy.listener("proxy");

        let mut options = DaemonWithMaildirOptions::new()
            .policy_file("source_with_proxy.lua")
            .env("KUMO_PROXY_SERVER_ADDRESS", proxy_addr.to_string());

        // Add any additional environment variables
        for (key, value) in kumod_env {
            options = options.env(key, value);
        }

        let kumod = options
            .start()
            .await
            .context("DaemonWithMaildirOptions::start")?;

        Ok(Self { kumod, proxy })
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        self.kumod.stop_both().await.context("stop kumod")?;
        self.proxy.stop().await.context("stop proxy")?;
        Ok(())
    }
}

// =============================================================================
// Test 1: Legacy mode still works without any visible change to existing users
// =============================================================================

#[tokio::test]
async fn proxy_legacy_mode() -> anyhow::Result<()> {
    let mut daemon = ProxyDaemon::spawn(ProxyArgs {
        proxy_config: None,
        env: vec![],
    })
    .await?;

    // Verify we got a listener
    let addr = daemon.listener("proxy");
    assert!(addr.port() > 0);

    daemon.stop().await?;
    Ok(())
}

// =============================================================================
// Test 2: Proxy server starts with Lua config (--proxy-config works)
// =============================================================================

#[tokio::test]
async fn proxy_with_lua_config() -> anyhow::Result<()> {
    let mut daemon = ProxyDaemon::spawn(ProxyArgs {
        proxy_config: Some("proxy_with_auth.lua".to_string()),
        env: vec![],
    })
    .await?;

    let addr = daemon.listener("proxy");
    assert!(addr.port() > 0);

    daemon.stop().await?;
    Ok(())
}

// =============================================================================
// Test 3: End-to-end test with source -> proxy -> sink (no auth)
// =============================================================================

#[tokio::test]
async fn proxy_end_to_end_noauth() -> anyhow::Result<()> {
    let mut daemon = DaemonWithProxy::spawn(
        ProxyArgs {
            proxy_config: Some("proxy_with_auth.lua".to_string()),
            env: vec![],
        },
        vec![],
    )
    .await
    .context("DaemonWithProxy::spawn")?;

    eprintln!("sending message through proxy");
    let mut client = daemon
        .kumod
        .smtp_client()
        .await
        .context("make smtp_client")?;

    let response = MailGenParams::default()
        .send(&mut client)
        .await
        .context("send message")?;

    eprintln!("response: {response:?}");
    anyhow::ensure!(response.code == 250, "expected 250, got {}", response.code);

    // Wait for delivery
    daemon
        .kumod
        .wait_for_maildir_count(1, Duration::from_secs(50))
        .await;

    daemon.stop().await?;
    Ok(())
}

// =============================================================================
// Test 4: End-to-end test with authentication (positive case - valid creds)
// =============================================================================

#[tokio::test]
async fn proxy_end_to_end_auth_positive() -> anyhow::Result<()> {
    let mut daemon = DaemonWithProxy::spawn(
        ProxyArgs {
            proxy_config: Some("proxy_with_auth.lua".to_string()),
            env: vec![
                ("KUMO_PROXY_REQUIRE_AUTH".to_string(), "true".to_string()),
                (
                    "KUMO_PROXY_AUTH_USERNAME".to_string(),
                    "testuser".to_string(),
                ),
                (
                    "KUMO_PROXY_AUTH_PASSWORD".to_string(),
                    "testpass".to_string(),
                ),
            ],
        },
        vec![
            ("KUMO_PROXY_USERNAME".to_string(), "testuser".to_string()),
            ("KUMO_PROXY_PASSWORD".to_string(), "testpass".to_string()),
        ],
    )
    .await
    .context("DaemonWithProxy::spawn")?;

    eprintln!("sending message through proxy with valid auth");
    let mut client = daemon
        .kumod
        .smtp_client()
        .await
        .context("make smtp_client")?;

    let response = MailGenParams::default()
        .send(&mut client)
        .await
        .context("send message")?;

    eprintln!("response: {response:?}");
    anyhow::ensure!(response.code == 250, "expected 250, got {}", response.code);

    // Wait for delivery
    daemon
        .kumod
        .wait_for_maildir_count(1, Duration::from_secs(50))
        .await;

    daemon.stop().await?;
    Ok(())
}

// =============================================================================
// Test 5: End-to-end test with authentication (negative case - invalid creds)
// This test verifies that invalid credentials are rejected
// =============================================================================

#[tokio::test]
async fn proxy_end_to_end_auth_negative() -> anyhow::Result<()> {
    let mut daemon = DaemonWithProxy::spawn(
        ProxyArgs {
            proxy_config: Some("proxy_with_auth.lua".to_string()),
            env: vec![
                ("KUMO_PROXY_REQUIRE_AUTH".to_string(), "true".to_string()),
                (
                    "KUMO_PROXY_AUTH_USERNAME".to_string(),
                    "testuser".to_string(),
                ),
                (
                    "KUMO_PROXY_AUTH_PASSWORD".to_string(),
                    "testpass".to_string(),
                ),
            ],
        },
        vec![
            // Wrong credentials!
            ("KUMO_PROXY_USERNAME".to_string(), "wronguser".to_string()),
            ("KUMO_PROXY_PASSWORD".to_string(), "wrongpass".to_string()),
        ],
    )
    .await
    .context("DaemonWithProxy::spawn")?;

    eprintln!("sending message through proxy with invalid auth");
    let mut client = daemon
        .kumod
        .smtp_client()
        .await
        .context("make smtp_client")?;

    let response = MailGenParams::default()
        .send(&mut client)
        .await
        .context("send message")?;

    eprintln!("response: {response:?}");
    // Message should be accepted by the source kumod, but delivery will fail
    // because proxy auth fails. The message will be queued but not delivered.

    // Wait for transient failure to be logged (auth failure shows as transient failure)
    daemon
        .kumod
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    // Verify NO messages were delivered (auth should have failed)
    let delivery_summary = daemon.kumod.dump_logs().await?;
    anyhow::ensure!(
        delivery_summary
            .source_counts
            .get(&TransientFailure)
            .copied()
            .unwrap_or(0)
            > 0,
        "expected transient failure count > 0"
    );
    anyhow::ensure!(
        delivery_summary
            .source_counts
            .get(&Delivery)
            .copied()
            .unwrap_or(0)
            == 0,
        "expected delivery count == 0"
    );

    daemon.stop().await?;
    Ok(())
}

// =============================================================================
// Test 6: require_auth validation - reject clients without auth capability
// =============================================================================

#[tokio::test]
async fn proxy_require_auth_rejects_noauth_client() -> anyhow::Result<()> {
    let mut daemon = DaemonWithProxy::spawn(
        ProxyArgs {
            proxy_config: Some("proxy_with_auth.lua".to_string()),
            env: vec![("KUMO_PROXY_REQUIRE_AUTH".to_string(), "true".to_string())],
        },
        // No auth credentials provided - client won't offer UsernamePassword
        vec![],
    )
    .await
    .context("DaemonWithProxy::spawn")?;

    eprintln!("sending message through proxy without auth (should fail)");
    let mut client = daemon
        .kumod
        .smtp_client()
        .await
        .context("make smtp_client")?;

    let response = MailGenParams::default()
        .send(&mut client)
        .await
        .context("send message")?;

    eprintln!("response: {response:?}");

    // Wait for transient failure to be logged (require_auth rejection shows as transient failure)
    daemon
        .kumod
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    // Verify transient failures occurred (require_auth rejected the client)
    let delivery_summary = daemon.kumod.dump_logs().await?;
    anyhow::ensure!(
        delivery_summary
            .source_counts
            .get(&TransientFailure)
            .copied()
            .unwrap_or(0)
            > 0,
        "expected transient failure count > 0 when require_auth rejects client"
    );
    anyhow::ensure!(
        delivery_summary
            .source_counts
            .get(&Delivery)
            .copied()
            .unwrap_or(0)
            == 0,
        "expected delivery count == 0"
    );

    daemon.stop().await?;
    Ok(())
}

// =============================================================================
// Test 7: Optional auth - client with auth succeeds even when not required
// =============================================================================

#[tokio::test]
async fn proxy_optional_auth_client_offers_auth() -> anyhow::Result<()> {
    let mut daemon = DaemonWithProxy::spawn(
        ProxyArgs {
            proxy_config: Some("proxy_with_auth.lua".to_string()),
            env: vec![
                // require_auth is false, but we still have an auth handler
                ("KUMO_PROXY_REQUIRE_AUTH".to_string(), "false".to_string()),
                (
                    "KUMO_PROXY_AUTH_USERNAME".to_string(),
                    "testuser".to_string(),
                ),
                (
                    "KUMO_PROXY_AUTH_PASSWORD".to_string(),
                    "testpass".to_string(),
                ),
            ],
        },
        vec![
            // Client offers valid auth
            ("KUMO_PROXY_USERNAME".to_string(), "testuser".to_string()),
            ("KUMO_PROXY_PASSWORD".to_string(), "testpass".to_string()),
        ],
    )
    .await
    .context("DaemonWithProxy::spawn")?;

    eprintln!("sending message with optional auth (client offers creds)");
    let mut client = daemon
        .kumod
        .smtp_client()
        .await
        .context("make smtp_client")?;

    let response = MailGenParams::default()
        .send(&mut client)
        .await
        .context("send message")?;

    eprintln!("response: {response:?}");
    anyhow::ensure!(response.code == 250, "expected 250, got {}", response.code);

    // Wait for delivery
    daemon
        .kumod
        .wait_for_maildir_count(1, Duration::from_secs(50))
        .await;

    daemon.stop().await?;
    Ok(())
}
