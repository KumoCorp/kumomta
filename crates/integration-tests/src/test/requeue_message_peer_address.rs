use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::TransientFailure;
use serde_json::Value;
use std::time::Duration;

#[tokio::test]
async fn requeue_event_peer_address() -> anyhow::Result<()> {
    for mode in ["smtp", "setup_failure", "legacy"] {
        let mut options = DaemonWithMaildirOptions::new()
            .env("KUMOD_TEST_REQUEUE_PEER_ADDRESS", mode)
            .env("KUMOD_RETRY_INTERVAL", "1s");
        if mode == "setup_failure" {
            options = options
                .env("KUMOD_SMTP_AUTH_USERNAME", "daniel")
                .env("KUMOD_SMTP_AUTH_PASSWORD", "incorrect-password");
        }
        let mut daemon = options.start().await?;
        let message = MailGenParams {
            recip: Some("tempfail@example.com"),

            ..Default::default()
        };
        let mut client = daemon.smtp_client().await?;
        assert_eq!(message.send(&mut client).await?.code, 250);
        assert!(
            daemon
                .wait_for_source_summary(
                    |summary| summary.get(&TransientFailure).copied().unwrap_or_default() >= 2,
                    Duration::from_secs(30),
                )
                .await,
            "{mode}: expected a retry after the first transient failure"
        );
        daemon.stop_both().await?;

        let logs = daemon.source.collect_logs().await?;
        let record = logs
            .iter()
            .find(|record| record.meta.contains_key("test_requeue_peer_address"))
            .with_context(|| format!("{mode}: missing callback metadata after retry"))?;
        let peer = record.meta["test_requeue_peer_address"]
            .as_str()
            .context("peer address metadata must be a string")?;
        match mode {
            "smtp" => {
                let peer: Value = serde_json::from_str(peer)?;
                let logged_peer = record
                    .peer_address
                    .as_ref()
                    .context("missing SMTP peer in disposition record")?;
                assert_eq!(peer, serde_json::to_value(logged_peer)?);
                assert_eq!(logged_peer.name, "localhost");
            }
            "setup_failure" => assert_eq!(peer, "none"),
            "legacy" => assert_eq!(peer, "legacy"),
            _ => unreachable!(),
        }
    }
    Ok(())
}
