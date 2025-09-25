use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::RecordType;
use kumo_log_types::RecordType::Bounce;
use std::time::Duration;

#[tokio::test]
async fn eightbit_yes_8bitmime() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        full_content: Some("Subject: 👾\r\n\r\ninvader\r\n"),
        ignore_8bit_checks: true,
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );

    let mut messages = daemon.extract_maildir_messages()?;

    assert_equal!(messages.len(), 1);
    let body = String::from_utf8_lossy(messages[0].read_data().unwrap());
    assert!(
        body.contains("👾"),
        "expected to see invader emoji in {body}"
    );

    Ok(())
}

#[tokio::test]
async fn eightbit_no_8bitmime() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_HIDE_8BITMIME", "1")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        full_content: Some("Subject: 👾\r\n\r\ninvader\r\n"),
        ignore_8bit_checks: true,
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Bounce).copied().unwrap_or(0) > 0,
            Duration::from_secs(10),
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
        Bounce: 1,
    },
    sink_counts: {},
}
"
    );

    let records = daemon.source.collect_logs().await?;
    let mut bounces = vec![];
    for record in records {
        if record.kind == RecordType::Bounce {
            bounces.push(format!(
                "from=<{}> to=<{}> why='{}' subject={}",
                record.sender,
                record.recipient,
                record.response.content,
                record.headers.get("Subject").unwrap().to_string(),
            ));
        }
    }

    k9::snapshot!(
        bounces,
        r#"
[
    "from=<sender@example.com> to=<recip@example.com> why='KumoMTA internal: DATA is 8bit, destination does not support 8BITMIME. Conversion via msg:check_fix_conformance during reception is required' subject="👾"",
]
"#
    );

    Ok(())
}

#[tokio::test]
async fn eightbit_yes_smtputf8() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        full_content: Some("Subject: plain\r\n\r\ninvader\r\n"),
        recip: Some("👾@example.com"),
        ignore_8bit_checks: true,
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);

    Ok(())
}

#[tokio::test]
async fn eightbit_no_smtputf8() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_HIDE_SMTPUTF8", "1")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        full_content: Some("Subject: plain\r\n\r\ninvader\r\n"),
        recip: Some("👾@example.com"),
        ignore_8bit_checks: true,
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Bounce: 1,
    },
    sink_counts: {},
}
"
    );

    let records = daemon.source.collect_logs().await?;
    let mut bounces = vec![];
    for record in records {
        if record.kind == RecordType::Bounce {
            bounces.push(format!(
                "from=<{}> to=<{}> why='{}' subject={}",
                record.sender,
                record.recipient,
                record.response.content,
                record.headers.get("Subject").unwrap().to_string(),
            ));
        }
    }

    k9::snapshot!(
        bounces,
        r#"
[
    "from=<sender@example.com> to=<👾@example.com> why='KumoMTA internal: envelope is 8bit, destination does not support SMTPUTF8.' subject="plain"",
]
"#
    );

    Ok(())
}
