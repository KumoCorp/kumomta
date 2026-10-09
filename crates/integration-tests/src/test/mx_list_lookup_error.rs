use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use kumo_log_types::RecordType::Delivery;
use serde_json::json;
use std::time::Duration;
use tokio::net::UdpSocket;

/// An explicit mx_list must retain healthy alternatives when another entry's
/// address lookup errors, regardless of where that entry appears in the list.
#[tokio::test]
async fn mx_list_lookup_error_keeps_healthy_alternatives() -> anyhow::Result<()> {
    // Keep the port reserved, but never reply to DNS queries sent to it. All
    // DNS/SMTP traffic is loopback-only. The healthy entry is already resolved
    // and doesn't need DNS.
    let dns_server = UdpSocket::bind("127.0.0.1:0").await?;
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-mxlist.lua")
        .env(
            "KUMOD_TEST_DNS_TIMEOUT_SERVER",
            &dns_server.local_addr()?.to_string(),
        )
        .start()
        .await?;
    let healthy = json!({
        "name": "healthy.example.test",
        "addr": daemon.sink.listener("smtp").to_string(),
    });
    let unavailable = format!(
        "unavailable.example.test:{}",
        daemon.sink.listener("smtp").port()
    );
    std::fs::write(
        daemon.source.dir.path().join("queue-data.json"),
        serde_json::to_vec(&json!({
            "control.example.test": [healthy],
            "error-first.example.test": [unavailable, healthy],
            "error-last.example.test": [healthy, unavailable],
        }))?,
    )?;

    let mut client = daemon.smtp_client().await?;
    let response = MailGenParams {
        recip: Some("recip@control.example.test"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);
    anyhow::ensure!(
        daemon
            .wait_for_maildir_count(1, Duration::from_secs(10))
            .await,
        "the healthy-only control must deliver before testing lookup failures"
    );

    for recipient in [
        "recip@error-first.example.test",
        "recip@error-last.example.test",
    ] {
        let response = MailGenParams {
            recip: Some(recipient),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250);
    }
    daemon
        .wait_for_maildir_count(3, Duration::from_secs(10))
        .await;
    daemon.stop_both().await?;

    let mut delivered: Vec<_> = daemon
        .source
        .collect_logs()
        .await?
        .into_iter()
        .filter(|record| record.kind == Delivery)
        .flat_map(|record| record.recipient)
        .collect();
    delivered.sort();
    k9::assert_equal!(
        delivered,
        vec![
            "recip@control.example.test".to_string(),
            "recip@error-first.example.test".to_string(),
            "recip@error-last.example.test".to_string(),
        ],
        "a failed mx_list lookup must not discard the healthy alternative"
    );
    Ok(())
}
