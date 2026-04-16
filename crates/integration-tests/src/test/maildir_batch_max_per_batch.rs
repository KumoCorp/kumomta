use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::RecordType;
use std::time::Duration;

#[tokio::test]
async fn maildir_batch_max_per_batch() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_BATCH_HANDLING", "BatchByDomain")
        .env("KUMOD_MAX_RECIPIENTS_PER_BATCH", "1")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let status = MailGenParams {
        recip_list: Some(vec![
            "fred@example.com",
            "frank@example.com",
            "joe@example.com",
            "pete@example.com",
            "john@example.com",
        ]),
        ..Default::default()
    }
    .send_batch(&mut client)
    .await
    .context("send message")?;
    eprintln!("{status:?}");
    anyhow::ensure!(status.response.code == 250);

    daemon
        .wait_for_maildir_count(5, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 5,
    },
    sink_counts: {
        Reception: 5,
        Delivery: 5,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 5,
    delivered: 5,
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 5);

    // Check the batch composition.
    let logs = daemon.source.collect_logs().await?;
    let mut batch_recips: Vec<Vec<String>> = vec![];
    for record in logs {
        if RecordType::Delivery == record.kind {
            batch_recips.push(record.recipient);
        }
    }

    batch_recips.sort();

    k9::assert_equal!(
        batch_recips,
        vec![
            vec!["frank@example.com".to_string()],
            vec!["fred@example.com".to_string()],
            vec!["joe@example.com".to_string()],
            vec!["john@example.com".to_string()],
            vec!["pete@example.com".to_string()],
        ]
    );

    // and ensure that it matches what we actually saw in the sink
    let logs = daemon.sink.collect_logs().await?;
    let mut batch_recips: Vec<Vec<String>> = vec![];
    for record in logs {
        if RecordType::Reception == record.kind {
            batch_recips.push(record.recipient);
        }
    }

    batch_recips.sort();

    k9::assert_equal!(
        batch_recips,
        vec![
            vec!["frank@example.com".to_string()],
            vec!["fred@example.com".to_string()],
            vec!["joe@example.com".to_string()],
            vec!["john@example.com".to_string()],
            vec!["pete@example.com".to_string()],
        ]
    );

    Ok(())
}
