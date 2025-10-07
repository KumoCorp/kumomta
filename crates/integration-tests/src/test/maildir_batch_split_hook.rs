use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::RecordType;
use std::time::Duration;

#[tokio::test]
async fn maildir_batch_split_hook() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        // KUMOD_USE_SPLIT_TXN enables a smtp_server_split_transaction event
        // handler that will group based on first letter of local part + domain
        .env("KUMOD_USE_SPLIT_TXN", "1")
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
        Reception: 3,
        Delivery: 3,
    },
    sink_counts: {
        Reception: 3,
        Delivery: 3,
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
    // Since we set KUMOD_USE_SPLIT_TXN the smtp_server_split_transaction
    // implementation will use the first letter of the local part and
    // the domain to group recipients together.  So we expect to see
    // `fXXX@` in one batch, `jXXX@` in another and so on.
    let logs = daemon.source.collect_logs().await?;
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
            vec![
                "fred@example.com".to_string(),
                "frank@example.com".to_string(),
            ],
            vec![
                "joe@example.com".to_string(),
                "john@example.com".to_string(),
            ],
            vec!["pete@example.com".to_string()],
        ]
    );

    Ok(())
}
