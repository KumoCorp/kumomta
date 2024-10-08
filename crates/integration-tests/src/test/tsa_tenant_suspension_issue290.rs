use crate::kumod::{DaemonWithTsa, MailGenParams};
use chrono::Utc;
use kumo_api_types::SuspendV1ListEntry;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

#[tokio::test]
async fn tsa_tenant_suspension_issue290() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;

    // send a message that will trigger a suspension rule.
    let response = MailGenParams {
        full_content: Some("X-Tenant: mytenant\r\n\r\nFoo"),
        recip: Some("450-suspend-tenant@foo.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

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

    // Insert a message that would normally go straight through,
    // but set its first attempt time to a few seconds in the future,
    // causing it to go through the scheduled queue before being inserted
    // into the ready queue. This was the problematic case in the original
    // issue.
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

    daemon
        .with_maildir
        .wait_for_source_summary(
            |summary| {
                summary.get(&TransientFailure).copied().unwrap_or(0) > 1
                    && summary.get(&Delivery).copied().unwrap_or(0) > 1
            },
            Duration::from_secs(10),
        )
        .await;

    let delivery_summary = daemon.with_maildir.dump_logs()?;
    // Note that the 2 rejections here are from the original message;
    // the first is the policy rejection, the second is a rejection
    // logged about DATA requiring a transaction that is triggered because
    // the source is using pipelining and cannot conditionally decide
    // not to send the DATA portion based on the policy rejection.
    // So we're really looking at a single actual rejection from the
    // sink sice, but two transient failures when sending.
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

    daemon.stop().await?;
    Ok(())
}
