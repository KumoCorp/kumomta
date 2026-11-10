use crate::kumod::{DaemonWithMaildir, KumoArgs, MailGenParams};
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

/// A message whose per-message expiry is already in the past when it is
/// received must expire immediately, before any delivery attempt.
#[tokio::test]
async fn already_expired_on_receipt() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let now = Utc::now();
    let schedule = Scheduling {
        restriction: None,
        first_attempt: None,
        expires: Some((now - chrono::Duration::seconds(10)).into()),
    };
    let schedule = serde_json::to_string(&schedule).unwrap();
    let body = format!(
        r#"Subject: hello
X-Schedule: {schedule}

Hello
"#
    );
    let response = MailGenParams {
        recip: Some("allow@example.com"),
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

    // The message expires at receipt. The sink never sees a delivery attempt.
    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Expiration: 1,
    },
    sink_counts: {},
}
"
    );

    Ok(())
}

/// A message held in the spool whose per-message expiry lapses while the
/// daemon is down must be expired when the spool is reloaded, not delivered.
/// A message's expiry must hold regardless of what kept it from being
/// delivered before the deadline, including a restart of the daemon.
#[tokio::test]
async fn expired_while_spooled_is_not_delivered_after_restart() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let now = Utc::now();
    // first_attempt must stay far enough out that the message is still
    // waiting in the scheduled queue, not yet due, when we stop the daemon
    // a few lines down.
    let first_attempt = now + chrono::Duration::seconds(15);
    // Still in the future at receipt.
    let expires = now + chrono::Duration::seconds(17);
    let schedule = Scheduling {
        restriction: None,
        first_attempt: Some(first_attempt.into()),
        expires: Some(expires.into()),
    };
    let schedule = serde_json::to_string(&schedule).unwrap();
    let body = format!(
        r#"Subject: hello
X-Schedule: {schedule}

Hello
"#
    );
    let response = MailGenParams {
        recip: Some("allow@example.com"),
        full_content: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250);

    // Flush the spool to on-disk SSTs.
    for name in ["data", "meta"] {
        daemon
            .source
            .kcli_text(["spool-compact", "--name", name])
            .await
            .with_context(|| format!("spool-compact {name}"))?;
    }
    // Stop before first_attempt is due.
    let sink_smtp_port = daemon.sink.listener("smtp").port();
    let sink_http_port = daemon.sink.listener("http").port();
    let stopped = daemon
        .source
        .stop_temporarily()
        .await
        .context("stop source")?;

    // Wait until the per-message expiry has elapsed before restarting.
    let wake = expires + chrono::Duration::seconds(3);
    let remaining = (wake - Utc::now()).to_std().unwrap_or(Duration::ZERO);
    tokio::time::sleep(remaining).await;

    daemon.source = stopped
        .start(KumoArgs {
            policy_file: "source.lua".to_string(),
            env: vec![
                (
                    "KUMOD_SMTP_SINK_PORT".to_string(),
                    sink_smtp_port.to_string(),
                ),
                (
                    "KUMOD_HTTP_SINK_PORT".to_string(),
                    sink_http_port.to_string(),
                ),
            ],
        })
        .await
        .context("restart source")?;

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Expiration).copied().unwrap_or(0) > 0,
            Duration::from_secs(30),
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
        Expiration: 1,
        Delayed: 1,
    },
    sink_counts: {},
}
"
    );

    Ok(())
}
