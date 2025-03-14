use crate::kumod::{DaemonWithTsa, MailGenParams};
use kumo_api_types::SuspendV1ListEntry;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

#[tokio::test]
async fn tsa_campaign_suspension() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;

    let body = "X-Tenant: mytenant\r\nX-Campaign: mycamp\r\n\r\nFoo";
    let response = MailGenParams {
        full_content: Some(body),
        recip: Some("450-suspend-campaign@foo.mx-sink.wezfurlong.org"),
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
            Duration::from_secs(50),
        )
        .await;

    let delivery_summary = daemon.with_maildir.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
        TransientFailure: 1,
    },
    sink_counts: {
        Rejection: 2,
    },
}
"
    );

    async fn get_suspensions(daemon: &DaemonWithTsa) -> anyhow::Result<Vec<SuspendV1ListEntry>> {
        daemon.with_maildir.kcli_json(["suspend-list"]).await
    }

    for _ in 0..5 {
        let status = get_suspensions(&daemon).await?;
        if status.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        } else {
            break;
        }
    }

    // Confirm kumod sees a suspension
    let status = get_suspensions(&daemon).await?;
    println!("kcli status: {status:?}");

    assert!(!status.is_empty(), "did suspend");
    let item = &status[0];
    let reason = &item.reason;
    assert!(reason.contains("you said"), "{reason}");
    assert_eq!(item.campaign.as_deref(), Some("mycamp"));
    assert_eq!(item.tenant.as_deref(), Some("mytenant"));
    assert!(
        item.duration.as_secs() > 50 * 60,
        "expiration should be about an hour remaining, {item:?}"
    );

    daemon.stop().await?;
    Ok(())
}
