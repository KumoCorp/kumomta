use crate::kumod::{DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use kumo_log_types::ResolvedAddress;
use std::time::Duration;

#[tokio::test]
async fn disconnect_in_data() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams {
        recip: Some("not-me@two-hosts.example.com"),
        // Cause the sink to 451 disconnect us in DATA.
        // This is to verify that we handle this sort of error during
        // a pipeline send correctly.
        sender: Some("disconnect-in-data-no-421@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    let response = MailGenParams {
        recip: Some("pick-me@two-hosts.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(10),
        )
        .await;

    daemon.stop_both().await?;

    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 2,
        Delivery: 1,
        TransientFailure: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
        Rejection: 1,
    },
}
"
    );

    Ok(())
}

#[tokio::test]
async fn disconnect_in_data_try_next() -> anyhow::Result<()> {
    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_TRY_NEXT_HOST_ON_TRANSPORT_ERROR", "1")])
            .await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams {
        recip: Some("not-me@two-hosts.example.com"),
        // Cause the sink to 451 disconnect us in mail from.
        // This is to verify that we handle this sort of error during
        // a pipeline send correctly.
        sender: Some("disconnect-in-data-no-421@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    let response = MailGenParams {
        recip: Some("pick-me@two-hosts.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&TransientFailure).copied().unwrap_or(0) > 0
                    && summary.get(&Delivery).copied().unwrap_or(0) > 0
            },
            Duration::from_secs(10),
        )
        .await;

    daemon.stop_both().await?;

    let logs = daemon.source.collect_logs().await?;

    // We want to see a TransientFailure for the first message, and a Delivery
    // for the second message, both with the same session id.
    let tf = logs
        .iter()
        .find(|record| record.kind == TransientFailure)
        .expect("at least one TransientFailure");
    let delivery = logs
        .iter()
        .find(|record| record.kind == Delivery)
        .expect("at least one Delivery");

    // Same session, but different hosts is what we expect: it indicates
    // that the first message snipped the connection and that the second
    // one used the next option
    k9::assert_equal!(tf.session_id, delivery.session_id);
    assert_ne!(tf.peer_address, delivery.peer_address);

    k9::snapshot!(
        &tf.response,
        r#"
Response {
    code: 451,
    enhanced_code: None,
    content: "disconnecting disconnect-in-data-no-421",
    command: Some(
        ".\r
",
    ),
}
"#
    );

    assert_eq!(
        tf.peer_address,
        Some(ResolvedAddress {
            name: "localhost-1".to_string(),
            addr: daemon.sink.listener("smtp").into(),
        })
    );
    assert_eq!(
        delivery.peer_address,
        Some(ResolvedAddress {
            name: "localhost-2".to_string(),
            addr: daemon.sink.listener("smtp").into(),
        })
    );

    Ok(())
}
