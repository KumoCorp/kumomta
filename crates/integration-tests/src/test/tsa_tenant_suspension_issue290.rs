use crate::kumod::{DaemonWithTsa, MailGenParams};
use chrono::Utc;
use kumo_api_types::SuspendV1ListEntry;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

#[tokio::test]
async fn tsa_tenant_suspension_issue290() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;

    // Insert a message that would normally go straight through,
    // but set its first attempt time to a few seconds in the future,
    // causing it to be attempted after the message that we're going
    // to send in a moment.
    let attempt = (Utc::now() + chrono::Duration::seconds(3)).to_rfc3339();
    let response = MailGenParams {
        full_content: Some(&format!(
            "X-Tenant: mytenant\r\nX-Schedule: {{\"first_attempt\": \"{attempt}\"}}\r\n\r\nFoo"
        )),
        recip: Some("allowme@foo.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    // send a message that will trigger a suspension rule.
    // We expect it to hit both this message and the one we injected
    // earlier, but will move from the scheduled queue after we've
    // tried this one and triggered the suspension
    let response = MailGenParams {
        full_content: Some("X-Tenant: mytenant\r\n\r\nFoo"),
        recip: Some("450-suspend-tenant@foo.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .with_maildir
        .wait_for_source_summary(
            |summary| {
                summary.get(&TransientFailure).copied().unwrap_or(0) > 1
                    && summary.get(&Delivery).copied().unwrap_or(0) > 1
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
        Reception: 2,
        Delivery: 2,
        TransientFailure: 2,
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
    assert_eq!(item.campaign.as_deref(), None);
    assert_eq!(item.tenant.as_deref(), Some("mytenant"));
    assert!(
        item.duration.as_secs() > 50 * 60,
        "expiration should be about an hour remaining, {item:?}"
    );

    daemon.stop().await?;
    Ok(())
}
