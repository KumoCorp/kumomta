use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use kumo_api_types::BounceV1ListEntry;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

#[tokio::test]
async fn tsa_bounce_automation() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("550-go-away@foo.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .with_maildir
        .wait_for_source_summary(
            |summary| {
                summary.get(&TransientFailure).copied().unwrap_or(0) > 0
                    && summary.get(&Delivery).copied().unwrap_or(0) > 0
            },
            Duration::from_secs(5),
        )
        .await;

    let delivery_summary = daemon.with_maildir.dump_logs()?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
        Bounce: 1,
    },
    sink_counts: {
        Rejection: 2,
    },
}
"
    );

    async fn get_bounces(daemon: &DaemonWithTsa) -> anyhow::Result<Vec<BounceV1ListEntry>> {
        daemon
            .with_maildir
            .kcli_json(["bounce-list", "--json"])
            .await
    }

    for _ in 0..5 {
        let status = get_bounces(&daemon).await?;
        if status.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        } else {
            break;
        }
    }

    // Confirm kumod sees a bounce
    let status = get_bounces(&daemon).await?;
    println!("kcli status: {status:?}");

    assert!(!status.is_empty(), "did bounce");
    let reason = &status[0].reason;
    assert!(reason.contains("you said"), "{reason}");
    let remaining = status[0].duration;
    assert!(
        remaining.as_secs() > 50 * 60,
        "expiration should be about an hour remaining, {remaining:?}"
    );

    daemon.stop().await?;
    Ok(())
}
