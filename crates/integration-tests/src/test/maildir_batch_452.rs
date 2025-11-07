use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

#[tokio::test]
async fn maildir_batch_452_a_ambiguous() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_BATCH_HANDLING", "BatchByDomain")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    // What we expect to happen is for part of the batch (recip1-4) to go through,
    // while full1 and recip5 will both be transfailed for different reasons;
    // recip5 will be eligible for immediately redelivery, so we'll try that
    // batch (full1, recip5) immediately.  full1 will transfail while recip5
    // will be delivered
    let status = MailGenParams {
        recip_list: Some(vec![
            "recip1@example.com",
            // full1 will cause a mailbox full response
            "full1@example.com",
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
        TransientFailure: 1,
    },
    sink_counts: {
        Reception: 2,
        Delivery: 2,
        Rejection: 3,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 6,
    delivered: 5,
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 5);

    Ok(())
}

#[tokio::test]
async fn maildir_batch_452_a_unambiguous() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_BATCH_HANDLING", "BatchByDomain")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    // What we expect to happen is for part of the batch (recip1-4) to go through,
    // while full1 and recip5 will both be transfailed for different reasons;
    // recip5 will be eligible for immediately redelivery, so we'll try that
    // batch (full1, recip5) immediately.  full1 will transfail while recip5
    // will be delivered
    let status = MailGenParams {
        recip_list: Some(vec![
            "recip1@example.com",
            // full1 will cause a mailbox full response
            "full-enh1@example.com",
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
        TransientFailure: 1,
    },
    sink_counts: {
        Reception: 2,
        Delivery: 2,
        Rejection: 3,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 6,
    delivered: 5,
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 5);

    Ok(())
}

#[tokio::test]
async fn maildir_batch_452_b() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_BATCH_HANDLING", "BatchByDomain")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    // We expect recip1,2 to go through in the first attempt.
    // full1 will have a mailbox full response and be transiently failed.
    // It will not be retried immediately
    let status = MailGenParams {
        recip_list: Some(vec![
            "recip1@example.com",
            // full1 will cause a mailbox full response
            "full-enh1@example.com",
            "recip2@example.com",
        ]),
        ..Default::default()
    }
    .send_batch(&mut client)
    .await
    .context("send message")?;
    eprintln!("{status:?}");
    anyhow::ensure!(status.response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
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
        TransientFailure: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
        Rejection: 3,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 3,
    delivered: 2,
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 2);

    Ok(())
}

#[tokio::test]
async fn maildir_batch_452_single() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_BATCH_HANDLING", "BatchByDomain")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let status = MailGenParams {
        recip_list: Some(vec![
            // full1 will cause a bare 452 which is ambiguous in the context
            // of a multi-recipient RCPT TO response. We want to see just
            // a single transient for this because it is a single recipient
            // batch and the ambiguity is removed
            "full1@example.com",
        ]),
        ..Default::default()
    }
    .send_batch(&mut client)
    .await
    .context("send message")?;
    eprintln!("{status:?}");
    anyhow::ensure!(status.response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(10),
        )
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
        TransientFailure: 1,
    },
    sink_counts: {
        Rejection: 2,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 1,
    delivered: 0,
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 0);

    Ok(())
}

#[tokio::test]
async fn maildir_batch_452_pathological() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_BATCH_HANDLING", "BatchByDomain")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let status = MailGenParams {
        recip_list: Some(vec![
            // full1 will cause a bare 452 which is ambiguous in the context
            // of a multi-recipient RCPT TO response.
            // This test has both recipients returning this ambiguous response
            // always, as a pathological case.
            // We want to see a bounded and reasonably small number of
            // attempts here: in this case 4 total, indicating that the
            // batch of 2 was retried once and no more than that.
            "full1@example.com",
            "full2@example.com",
        ]),
        ..Default::default()
    }
    .send_batch(&mut client)
    .await
    .context("send message")?;
    eprintln!("{status:?}");
    anyhow::ensure!(status.response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(10),
        )
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
        TransientFailure: 3,
    },
    sink_counts: {
        Rejection: 6,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 2,
    delivered: 0,
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 0);

    Ok(())
}
