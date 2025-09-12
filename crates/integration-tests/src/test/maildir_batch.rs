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
        recip_list: Some(vec!["recip1@example.com", "recip2@example.com"]),
        ..Default::default()
    }
    .send_batch(&mut client)
    .await
    .context("send message")?;
    eprintln!("{status:?}");
    anyhow::ensure!(status.response.code == 250);

    daemon
        .wait_for_maildir_count(2, Duration::from_secs(10))
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
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 2,
    delivered: 2,
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 2);

    Ok(())
}
