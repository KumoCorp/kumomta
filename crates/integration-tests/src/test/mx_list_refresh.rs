use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_api_types::InspectQueueV1Request;
use kumo_log_types::RecordType::Delivery;
use rfc5321::SmtpClient;
use serde_json::json;
use std::time::{Duration, Instant};

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
                {
                    "name": "1.1.sink",
                    "addr": sink_listener,
                },
                "255.255.255.255:1"
            ],
            "two.example.com": [
                {
                    "name": "2.1.sink",
                    "addr": sink_listener,
                },
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
                {
                    "name": "1.2.sink",
                    "addr": sink_listener,
                },
                "255.255.255.255:2"
            ],
            "two.example.com": [
                {
                    "name": "2.1.sink",
                    "addr": sink_listener,
                },
                "255.255.255.255:1"
            ],
        }))?,
    )?;

    daemon.source.api_client().admin_bump_config_epoch().await?;

    // Wait for the epoch bump to actually propagate into the
    // one.example.com queue's resolved config, rather than relying
    // on a fixed sleep that races under load.  Once the queue's
    // config reflects the new mx_list (the `255.255.255.255:2`
    // marker), or the queue has been reaped (in which case the
    // next send rebuilds it from the new config), we can proceed.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match daemon
            .source
            .api_client()
            .admin_inspect_sched_q_v1(&InspectQueueV1Request {
                queue_name: "one.example.com".to_string(),
                want_body: false,
                limit: Some(0),
            })
            .await
        {
            Ok(resp) => {
                if resp.queue_config.to_string().contains("255.255.255.255:2") {
                    break;
                }
            }
            // Queue not found: it was reaped, so the next send will
            // rebuild it from the freshly-loaded config.
            Err(_) => break,
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for one.example.com config refresh"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // In round 1 both queues share a ready queue (same resolved MX
    // addresses), so a single idle connection sits in that pool
    // tagged with one.example.com's mx hostname (`1.1.sink`).  After
    // the epoch bump, two.example.com's site is unchanged.  If we
    // dispatch round 2 before that idle connection has been reaped
    // it gets reused, and four@two.example.com ends up logged as
    // `1.1.sink` instead of `2.1.sink`.  Wait for the shared ready
    // queue's connection_count to reach 0 so the next send opens a
    // fresh connection picking up two.example.com's own mx_list.
    let shared_service =
        format!("smtp_client:unspecified->mx_list:{sink_listener},255.255.255.255:1@smtp_client");
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(30),
            |m| m.name().as_str() == "connection_count" && m.label_is("service", &shared_service),
            |conns| conns.iter().sum::<f64>() == 0.0,
        )
        .await
        .context("waiting for shared site connection pool to drain")?;

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
    let deliv_and_site: Vec<(String, String, String)> = logs
        .into_iter()
        .filter_map(|r| match r.kind {
            Delivery => Some((
                r.recipient[0].clone(),
                r.site.replace(&sink_listener, "SINK"),
                r.peer_address.unwrap().name,
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
        "1.1.sink",
    ),
    (
        "two@two.example.com",
        "unspecified->mx_list:SINK,255.255.255.255:1@smtp_client",
        "1.1.sink",
    ),
    (
        "three@one.example.com",
        "unspecified->mx_list:SINK,255.255.255.255:2@smtp_client",
        "1.2.sink",
    ),
    (
        "four@two.example.com",
        "unspecified->mx_list:SINK,255.255.255.255:1@smtp_client",
        "2.1.sink",
    ),
]
"#
    );

    Ok(())
}
