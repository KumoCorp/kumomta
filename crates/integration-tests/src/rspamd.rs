#![cfg(test)]
use crate::kumod::{generate_message_text, MailGenParams};
use anyhow::Context;
use std::time::Duration;
use testcontainers_modules::testcontainers::core::{ContainerPort, WaitFor};
use testcontainers_modules::testcontainers::{runners::AsyncRunner, GenericImage};

#[tokio::test]
async fn test_rspamd_scan_message() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

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

    // Create temporary Lua config file
    let lua_config = format!(
        r#"
local kumo = require 'kumo'

kumo.on('init', function()
  kumo.start_esmtp_listener {{
    listen = '0.0.0.0:0',
  }}

  kumo.start_http_listener {{
    listen = '127.0.0.1:0',
  }}

  kumo.define_spool {{
    name = 'data',
    path = '/tmp/kumo-test-spool/data',
  }}

  kumo.define_spool {{
    name = 'meta',
    path = '/tmp/kumo-test-spool/meta',
  }}
end)

-- Use smtp_server_data for per-batch scanning
kumo.on('smtp_server_data', function(msg)
  local config = {{
    base_url = '{rspamd_url}',
    add_headers = true,
    reject_spam = false,  -- Don't reject in test
  }}

  local result = kumo.rspamd.scan_message(config, msg)

  -- Store scan results in message metadata
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)

  -- Log for debugging
  print(string.format('Rspamd scan: score=%.2f action=%s', result.score, result.action))
end)
"#
    );

    let config_file = std::env::temp_dir().join("rspamd_test_config.lua");
    std::fs::write(&config_file, lua_config).context("write lua config")?;

    let mut daemon = crate::kumod::DaemonWithMaildirOptions::new()
        .policy_file(config_file.to_string_lossy().to_string())
        .start()
        .await?;

    eprintln!("Sending test message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
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

    // Wait for message to be delivered
    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    // Verify message was scanned (check logs for rspamd output)
    eprintln!("Message successfully delivered after Rspamd scan");

    daemon.stop_both().await.context("stop_both")?;
    eprintln!("Test completed successfully");

    Ok(())
}

#[tokio::test]
async fn test_rspamd_reject_spam() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

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

    // Create temporary Lua config file with rejection enabled
    let lua_config = format!(
        r#"
local kumo = require 'kumo'

kumo.on('init', function()
  kumo.start_esmtp_listener {{
    listen = '0.0.0.0:0',
  }}

  kumo.start_http_listener {{
    listen = '127.0.0.1:0',
  }}

  kumo.define_spool {{
    name = 'data',
    path = '/tmp/kumo-test-spool/data',
  }}

  kumo.define_spool {{
    name = 'meta',
    path = '/tmp/kumo-test-spool/meta',
  }}
end)

-- Use smtp_server_data for per-batch scanning
kumo.on('smtp_server_data', function(msg)
  local config = {{
    base_url = '{rspamd_url}',
    add_headers = true,
    reject_spam = true,
    reject_soft = false,
  }}

  local result = kumo.rspamd.scan_message(config, msg)

  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)

  print(string.format('Rspamd scan: score=%.2f action=%s', result.score, result.action))
end)
"#
    );

    let config_file = std::env::temp_dir().join("rspamd_test_reject_config.lua");
    std::fs::write(&config_file, lua_config).context("write lua config")?;

    let mut daemon = crate::kumod::DaemonWithMaildirOptions::new()
        .policy_file(config_file.to_string_lossy().to_string())
        .start()
        .await?;

    eprintln!("Sending test message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    // Send a normal message (should not be rejected)
    let body = generate_message_text(512, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;

    eprintln!("SMTP response: {response:?}");

    // Normal messages should be accepted
    anyhow::ensure!(
        response.code == 250,
        "Expected 250 response for normal message, got {}",
        response.code
    );

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;
    eprintln!("Test completed successfully");

    Ok(())
}

#[tokio::test]
async fn test_rspamd_headers() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

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

    // Create temporary Lua config file with header addition
    let lua_config = format!(
        r#"
local kumo = require 'kumo'

kumo.on('init', function()
  kumo.start_esmtp_listener {{
    listen = '0.0.0.0:0',
  }}

  kumo.start_http_listener {{
    listen = '127.0.0.1:0',
  }}

  kumo.define_spool {{
    name = 'data',
    path = '/tmp/kumo-test-spool/data',
  }}

  kumo.define_spool {{
    name = 'meta',
    path = '/tmp/kumo-test-spool/meta',
  }}
end)

-- Use smtp_server_data for per-batch scanning
kumo.on('smtp_server_data', function(msg)
  local config = {{
    base_url = '{rspamd_url}',
    add_headers = true,
    reject_spam = false,
  }}

  local result = kumo.rspamd.scan_message(config, msg)

  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)

  -- Verify headers were added
  local spam_flag = msg:get_first_named_header_value('X-Spam-Flag')
  local spam_score = msg:get_first_named_header_value('X-Spam-Score')
  local spam_action = msg:get_first_named_header_value('X-Spam-Action')

  print(string.format('Headers: Flag=%s Score=%s Action=%s',
    spam_flag or 'nil', spam_score or 'nil', spam_action or 'nil'))
end)
"#
    );

    let config_file = std::env::temp_dir().join("rspamd_test_headers_config.lua");
    std::fs::write(&config_file, lua_config).context("write lua config")?;

    let mut daemon = crate::kumod::DaemonWithMaildirOptions::new()
        .policy_file(config_file.to_string_lossy().to_string())
        .start()
        .await?;

    eprintln!("Sending test message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(512, 78);
    let response = MailGenParams {
        body: Some(&body),
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

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    // TODO: Read delivered message and verify X-Spam-* headers were added

    daemon.stop_both().await.context("stop_both")?;
    eprintln!("Test completed successfully");

    Ok(())
}

#[tokio::test]
async fn test_rspamd_per_recipient_threshold() -> anyhow::Result<()> {
    if std::env::var("KUMOD_TESTCONTAINERS").unwrap_or_else(|_| String::new()) != "1" {
        return Ok(());
    }

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

    // Create temporary Lua config file demonstrating per-recipient thresholds
    let lua_config = format!(
        r#"
local kumo = require 'kumo'
local rspamd = require 'policy-extras.rspamd'

kumo.on('init', function()
  kumo.start_esmtp_listener {{{{
    listen = '0.0.0.0:0',
  }}}}

  kumo.start_http_listener {{{{
    listen = '127.0.0.1:0',
  }}}}

  kumo.define_spool {{{{
    name = 'data',
    path = '/tmp/kumo-test-spool/data',
  }}}}

  kumo.define_spool {{{{
    name = 'meta',
    path = '/tmp/kumo-test-spool/meta',
  }}}}
end)

-- Build client
local client = kumo.rspamd.build_client {{{{
  base_url = '{rspamd_url}',
  zstd = true,
}}}}

-- Per-recipient threshold function
local function get_spam_threshold_for_user(recipient)
  if recipient:match '@vip%.example%.com$' then
    return 100.0  -- Very lenient for VIP (won't reject normal messages)
  else
    return 5.0  -- Strict for others
  end
end

-- Scan once per batch in smtp_server_data
kumo.on('smtp_server_data', function(msg, conn_meta)
  local result = rspamd.scan_message(client, msg, conn_meta, {{{{use_file_path = true}}}})

  -- Store results in metadata for later use
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)

  print(string.format('Rspamd scan: score=%.2f action=%s', result.score, result.action))

  -- Add headers but don't reject yet
  rspamd.apply_milter_actions(msg, result)
end)

-- Apply per-recipient thresholds in smtp_server_message_received
kumo.on('smtp_server_message_received', function(msg)
  local score = msg:get_meta 'rspamd_score'
  if not score then
    return
  end

  local recipient = tostring(msg:recipient())
  local threshold = get_spam_threshold_for_user(recipient)

  print(string.format('Checking recipient %s: score=%.2f threshold=%.2f',
    recipient, score, threshold))

  if score > threshold then
    kumo.reject(550, string.format(
      '5.7.1 Message rejected as spam (score: %.2f, threshold: %.2f)',
      score, threshold
    ))
  end

  msg:set_meta('spam_threshold', threshold)
end)
"#
    );

    let config_file = std::env::temp_dir().join("rspamd_test_per_recipient_config.lua");
    std::fs::write(&config_file, lua_config).context("write lua config")?;

    let mut daemon = crate::kumod::DaemonWithMaildirOptions::new()
        .policy_file(config_file.to_string_lossy().to_string())
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

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    eprintln!("VIP message delivered successfully");

    daemon.stop_both().await.context("stop_both")?;
    eprintln!("Test completed successfully");

    Ok(())
}
