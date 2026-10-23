use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use chrono::DateTime;
use kumo_api_types::SuspendReadyQueueV1ListEntry;
use kumo_log_types::RecordType::Delivery;
use regex::Regex;
use std::time::Duration;
use uuid::Uuid;

async fn get_suspensions(
    daemon: &DaemonWithTsa,
) -> anyhow::Result<Vec<SuspendReadyQueueV1ListEntry>> {
    daemon
        .with_maildir
        .kcli_json(["suspend-ready-q-list"])
        .await
}

async fn send_delivery_message(daemon: &mut DaemonWithTsa) -> anyhow::Result<()> {
    let mut client = daemon.smtp_client().await?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("success@delivery.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .with_maildir
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    Ok(())
}

// With the default skip_log_record_types, Delivery records flow through
// to TSA and can match automation rules.
#[tokio::test]
async fn tsa_skip_record_types_default_allows_delivery() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    send_delivery_message(&mut daemon).await?;

    let mut status = vec![];
    for _ in 0..10 {
        status = get_suspensions(&daemon).await?;
        if !status.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // Force time/id-varying fields to constants so we can do a full equality
    // assertion. The rule_hash embedded in the reason is normalized via regex.
    let rule_hash_re = Regex::new(r"rule_hash=[^)]+").expect("valid regex");
    let fixed_expires = DateTime::from_timestamp(0, 0).expect("valid timestamp");
    for entry in &mut status {
        entry.id = Uuid::nil();
        entry.expires = fixed_expires;
        entry.duration = Duration::from_secs(3600);
        entry.reason = rule_hash_re
            .replace_all(&entry.reason, "rule_hash=NORMALIZED")
            .into_owned();
    }

    k9::assert_equal!(
        status,
        vec![SuspendReadyQueueV1ListEntry {
            id: Uuid::nil(),
            name: "unspecified->localhost@smtp_client".to_string(),
            reason: "automation rule: OK ids= (rule_hash=NORMALIZED)".to_string(),
            duration: Duration::from_secs(3600),
            expires: fixed_expires,
        }]
    );

    daemon.stop().await?;
    Ok(())
}

// When skip_log_record_types is augmented to include Delivery, those
// records are filtered out at the source and never reach TSA, so the
// matching automation rule does not fire.
#[tokio::test]
async fn tsa_skip_record_types_skip_delivery() -> anyhow::Result<()> {
    let mut daemon =
        DaemonWithTsa::start_with_source_policy("tsa_source_skip_delivery.lua").await?;

    send_delivery_message(&mut daemon).await?;

    // Give the TSA pipeline ample time to process anything it might have
    // received; we expect nothing, so this is just to avoid a false negative.
    for _ in 0..10 {
        let status = get_suspensions(&daemon).await?;
        assert!(
            status.is_empty(),
            "no suspension expected when Delivery records are skipped, got {status:?}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    daemon.stop().await?;
    Ok(())
}
