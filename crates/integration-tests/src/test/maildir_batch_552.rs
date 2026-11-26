use crate::kumod::{DaemonWithMaildir, DaemonWithMaildirOptions, MailGenParams};
use kumo_api_types::InspectQueueV1Request;
use kumo_log_types::RecordType::{Bounce, Delivery, Reception, TransientFailure};
use std::time::Duration;

async fn send_batch(recipients: &[&str], source_policy: &str) -> anyhow::Result<DaemonWithMaildir> {
    let daemon = DaemonWithMaildirOptions::new()
        .policy_file(source_policy)
        .env("KUMOD_BATCH_HANDLING", "BatchByDomain")
        .sink_policy_file("maildir-sink-552.lua")
        .start()
        .await?;
    let mut client = daemon.smtp_client().await?;
    let result = MailGenParams {
        recip_list: Some(recipients.to_vec()),
        ..Default::default()
    }
    .send_batch(&mut client)
    .await?;
    assert_eq!(result.response.code, 250);
    Ok(daemon)
}

async fn check_first_refusal(accepted: &[&str]) -> anyhow::Result<()> {
    let refused = "full@example.com";
    let mut recipients = vec![refused];
    recipients.extend_from_slice(accepted);
    let mut daemon = send_batch(&recipients, "source.lua").await?;
    assert!(
        daemon
            .wait_for_source_summary(
                |s| s.contains_key(&TransientFailure)
                    || (s.contains_key(&Bounce)
                        && (accepted.is_empty() || s.contains_key(&Delivery))),
                Duration::from_secs(10)
            )
            .await
    );
    let queue = daemon
        .source
        .api_client()
        .admin_inspect_sched_q_v1(&InspectQueueV1Request {
            queue_name: "example.com".to_owned(),
            limit: Some(0),
            want_body: false,
        })
        .await?;
    daemon.stop_both().await?;
    let records = daemon.source.collect_logs().await?;
    assert!(
        !records.iter().any(|r| r.kind == TransientFailure),
        "first RCPT was deferred"
    );
    assert_eq!(queue.num_scheduled, 0);
    let bounces: Vec<_> = records.iter().filter(|r| r.kind == Bounce).collect();
    assert_eq!(bounces.len(), 1);
    assert_eq!(bounces[0].recipient, [refused]);
    assert_eq!(bounces[0].response.code, 552);
    assert_eq!(bounces[0].response.enhanced_code, None);
    assert_eq!(bounces[0].response.content, "recipient refused");
    let delivered: Vec<_> = records
        .iter()
        .filter(|r| r.kind == Delivery)
        .flat_map(|r| &r.recipient)
        .map(String::as_str)
        .collect();
    assert_eq!(delivered, accepted);
    Ok(())
}

#[tokio::test]
async fn sole_recipient_552_bounces() -> anyhow::Result<()> {
    check_first_refusal(&[]).await
}

#[tokio::test]
async fn first_recipient_552_does_not_affect_the_next_recipient() -> anyhow::Result<()> {
    check_first_refusal(&["next@example.com"]).await
}

async fn check_requeue_response(recipients: &[&str]) -> anyhow::Result<()> {
    let mut daemon = send_batch(recipients, "source-report-requeue.lua").await?;
    // One bounce for full@example.com and one from the requeue_message test policy.
    assert!(
        daemon
            .wait_for_source_summary(|s| s.get(&Bounce) == Some(&2), Duration::from_secs(10))
            .await
    );
    daemon.stop_both().await?;
    let records = daemon.source.collect_logs().await?;
    let policy_bounce = records
        .iter()
        .find(|r| r.kind == Bounce && r.response.code == 550)
        .expect("missing bounce from requeue_message");
    assert_eq!(policy_bounce.recipient, ["450-retry@example.com"]);
    assert_eq!(
        policy_bounce.response.content,
        "requeue_message saw: 450 you said 450-retry"
    );
    Ok(())
}

#[tokio::test]
async fn requeue_response_excludes_bounced_recipient() -> anyhow::Result<()> {
    check_requeue_response(&["full@example.com", "450-retry@example.com"]).await
}

#[tokio::test]
async fn requeue_response_excludes_successful_data_reply() -> anyhow::Result<()> {
    check_requeue_response(&[
        "full@example.com",
        "450-retry@example.com",
        "accepted@example.com",
    ])
    .await
}

#[tokio::test]
async fn legacy_552_overflow_retries_only_the_excess_recipient() -> anyhow::Result<()> {
    // The sink accepts four recipients per transaction.
    let recipients = [
        "a@example.com",
        "b@example.com",
        "c@example.com",
        "d@example.com",
        "e@example.com",
    ];
    let mut daemon = send_batch(&recipients, "source.lua").await?;
    assert!(
        daemon
            .wait_for_maildir_count(recipients.len(), Duration::from_secs(10))
            .await
    );
    daemon.stop_both().await?;
    let records = daemon.sink.collect_logs().await?;
    let mut batches: Vec<_> = records
        .iter()
        .filter(|r| r.kind == Reception)
        .map(|r| &r.recipient)
        .collect();
    batches.sort_by_key(|batch| batch.len());
    assert_eq!(batches, [&recipients[4..], &recipients[..4]]);
    Ok(())
}
