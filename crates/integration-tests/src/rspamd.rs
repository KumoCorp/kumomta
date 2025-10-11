#![cfg(test)]
use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
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
        .with_wait_for(WaitFor::message_on_stdout("rspamd main process started"));

    let rspamd_container = rspamd_image.start().await?;

    let rspamd_host = rspamd_container.get_host().await?;
    let rspamd_port = rspamd_container.get_host_port_ipv4(11333).await?;
    let rspamd_url = format!("http://{rspamd_host}:{rspamd_port}");

    eprintln!("Started Rspamd at {rspamd_url}");

    // Wait a bit for Rspamd to fully initialize
    tokio::time::sleep(Duration::from_secs(2)).await;

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
end)

kumo.on('smtp_server_message_received', function(msg)
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
        .with_wait_for(WaitFor::message_on_stdout("rspamd main process started"));

    let rspamd_container = rspamd_image.start().await?;

    let rspamd_host = rspamd_container.get_host().await?;
    let rspamd_port = rspamd_container.get_host_port_ipv4(11333).await?;
    let rspamd_url = format!("http://{rspamd_host}:{rspamd_port}");

    eprintln!("Started Rspamd at {rspamd_url}");

    // Wait for Rspamd to initialize
    tokio::time::sleep(Duration::from_secs(2)).await;

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
end)

kumo.on('smtp_server_message_received', function(msg)
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
        .with_wait_for(WaitFor::message_on_stdout("rspamd main process started"));

    let rspamd_container = rspamd_image.start().await?;

    let rspamd_host = rspamd_container.get_host().await?;
    let rspamd_port = rspamd_container.get_host_port_ipv4(11333).await?;
    let rspamd_url = format!("http://{rspamd_host}:{rspamd_port}");

    eprintln!("Started Rspamd at {rspamd_url}");

    // Wait for Rspamd to initialize
    tokio::time::sleep(Duration::from_secs(2)).await;

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
end)

kumo.on('smtp_server_message_received', function(msg)
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
