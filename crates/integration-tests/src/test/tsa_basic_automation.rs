use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use chrono::Utc;
use kumo_api_types::SuspendReadyQueueV1ListEntry;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;

#[tokio::test]
async fn tsa_basic_automation() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("450-go-away@foo.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .with_maildir
        .wait_for_source_summary(
            |summary| {
                summary.get(&TransientFailure).copied().unwrap_or(0) > 0
                    && summary.get(&Delivery).copied().unwrap_or(0) > 0
            },
            Duration::from_secs(50),
        )
        .await;

    let delivery_summary = daemon.with_maildir.dump_logs()?;
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
        Rejection: 2,
    },
}
"
    );

    async fn get_suspensions(
        daemon: &DaemonWithTsa,
    ) -> anyhow::Result<Vec<SuspendReadyQueueV1ListEntry>> {
        daemon
            .with_maildir
            .kcli_json(["suspend-ready-q-list"])
            .await
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
    assert_eq!(status[0].name, "unspecified->localhost@smtp_client");
    let reason = &status[0].reason;
    assert!(reason.contains("you said"), "{reason}");
    let remaining = status[0].expires - Utc::now();
    assert!(
        remaining.num_minutes() > 50,
        "expiration should be about an hour remaining, {remaining:?}"
    );

    let shaping = daemon.tsa.get_shaping().await?;
    eprintln!("{shaping:#?}");
    let partial = shaping
        .get_egress_path_config_value(
            "foo.mx-sink.wezfurlong.org",
            "unspecified",
            "loopback.dummy-mx.wezfurlong.org",
        )
        .await?;
    eprintln!("{partial:#?}");
    assert_eq!(partial.get("max_message_rate").unwrap(), "3/m");
    assert_eq!(partial.get("max_deliveries_per_connection").unwrap(), 1);

    daemon.stop().await?;
    Ok(())
}
