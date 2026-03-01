use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::Delivery;
use rfc5321::SmtpClient;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn mx_list_refresh() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-mxlist.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let queue_data_path = daemon.source.dir.path().join("queue-data.json");
    let sink_listener = daemon.sink.listener("smtp").to_string();
    std::fs::write(
        &queue_data_path,
        serde_json::to_string(&json!({
            "one.example.com": [
                sink_listener,
                "255.255.255.255:1"
            ],
            "two.example.com": [
                sink_listener,
                "255.255.255.255:1"
            ],
        }))?,
    )?;

    let mut client = daemon.smtp_client().await?;
    async fn send(client: &mut SmtpClient, recip: &str) -> anyhow::Result<()> {
        let response = MailGenParams {
            recip: Some(recip),
            ..Default::default()
        }
        .send(client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);
        Ok(())
    }

    send(&mut client, "one@one.example.com").await?;
    send(&mut client, "two@two.example.com").await?;

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 2,
            Duration::from_secs(5),
        )
        .await;

    // Update config
    std::fs::write(
        &queue_data_path,
        serde_json::to_string(&json!({
            "one.example.com": [
                sink_listener,
                "255.255.255.255:1"
            ],
            "two.example.com": [
                sink_listener,
                "255.255.255.255:2"
            ],
        }))?,
    )?;

    // daemon.source.api_client().admin_bump_config_epoch().await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(13)).await;

    send(&mut client, "three@one.example.com").await?;
    send(&mut client, "four@two.example.com").await?;

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 4,
            Duration::from_secs(5),
        )
        .await;

    daemon.stop_both().await?;
    daemon.assert_no_acct_deny().await?;
    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 4,
        Delivery: 4,
    },
    sink_counts: {
        Reception: 4,
        Delivery: 4,
    },
}
"
    );

    let logs = daemon.source.collect_logs().await?;
    let deliv_and_site: Vec<(String, String)> = logs
        .into_iter()
        .filter_map(|r| match r.kind {
            Delivery => Some((
                r.recipient[0].clone(),
                r.site.replace(&sink_listener, "SINK"),
            )),
            _ => None,
        })
        .collect();

    k9::snapshot!(
        deliv_and_site,
        r#"
[
    (
        "one@one.example.com",
        "unspecified->mx_list:SINK,255.255.255.255:1@smtp_client",
    ),
    (
        "two@two.example.com",
        "unspecified->mx_list:SINK,255.255.255.255:1@smtp_client",
    ),
    (
        "three@one.example.com",
        "unspecified->mx_list:SINK,255.255.255.255:1@smtp_client",
    ),
    (
        "four@two.example.com",
        "unspecified->mx_list:SINK,255.255.255.255:2@smtp_client",
    ),
]
"#
    );

    Ok(())
}
