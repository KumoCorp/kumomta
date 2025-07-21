use crate::kumod::{DaemonWithMaildir, MailGenParams};
use kumo_api_types::SuspendV1Response;
use kumo_log_types::RecordType::{Delivery, Reception};
use std::time::Duration;

#[tokio::test]
async fn rebind_port_routing_domain_timerwheel() -> anyhow::Result<()> {
    rebind_port_impl(
        "TimerWheel",
        "queue=rebound.example.com!localhost:SINK_PORT",
    )
    .await
}

#[tokio::test]
async fn rebind_port_domain_timerwheel() -> anyhow::Result<()> {
    rebind_port_impl("TimerWheel", "queue=localhost:SINK_PORT").await
}

async fn rebind_port_impl(strategy: &str, target_queue: &str) -> anyhow::Result<()> {
    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_QUEUE_STRATEGY", strategy)]).await?;
    let mut client = daemon.smtp_client().await?;

    let status: SuspendV1Response = daemon
        .kcli_json([
            "suspend",
            "--domain",
            "rebind.example.com",
            "--reason",
            "testing",
        ])
        .await?;
    println!("kcli status: {status:?}");

    let response = MailGenParams {
        recip: Some("allow@rebind.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    let sink_port = daemon.sink.listener("smtp").port();

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Reception).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    let data = target_queue.replace("SINK_PORT", &sink_port.to_string());

    daemon
        .kcli([
            "rebind",
            "--domain",
            "rebind.example.com",
            "--reason",
            "testing",
            "--set",
            &data,
        ])
        .await?;

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await?;
    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
        TransientFailure: 1,
        AdminRebind: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );
    Ok(())
}
