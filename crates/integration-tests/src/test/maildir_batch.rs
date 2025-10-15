use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

#[tokio::test]
async fn maildir_batch() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_BATCH_HANDLING", "BatchByDomain")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let status = MailGenParams {
        recip_list: Some(vec![
            "recip1@example.com",
            "recip2@example.com",
            "recip3@example.com",
            "recip4@example.com",
            // maildir-sink.lua is configured with max_recipients_per_message=4,
            // so we expect this additional recipient to be rejected in the
            // initial batch
            "recip5@example.com",
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
        Delivery: 2,
    },
    sink_counts: {
        Reception: 2,
        Delivery: 2,
        Rejection: 1,
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

    Ok(())
}
