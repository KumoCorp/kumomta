use crate::kumod::{DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use chrono::Utc;
use kumo_log_types::RecordType::Expiration;
use message::scheduling::Scheduling;
use std::time::Duration;

#[tokio::test]
async fn scheduling_header_expiry() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let now = Utc::now();
    // This schedule should result in 1 attempt (which the sink will transfail),
    // then an expiration for the next attempt
    let schedule = Scheduling {
        restriction: None,
        first_attempt: None,
        expires: Some((now + chrono::Duration::seconds(10)).into()),
    };
    let schedule = serde_json::to_string(&schedule).unwrap();
    let body = format!(
        r#"Subject: hello
X-Schedule: {schedule}

Hello
"#
    );
    let response = MailGenParams {
        recip: Some("tempfail@example.com"),
        full_content: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Expiration).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        TransientFailure: 1,
        Expiration: 1,
    },
    sink_counts: {
        Rejection: 2,
    },
}
"
    );

    Ok(())
}
