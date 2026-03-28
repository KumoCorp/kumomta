use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

#[tokio::test]
async fn bad_source_address() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("broken-source.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    let sink_port = daemon.sink.listener("smtp").port();

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) >= 1,
            Duration::from_secs(5),
        )
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    let transient: Vec<String> = logs
        .into_iter()
        .filter_map(|r| match r.kind {
            TransientFailure => Some(
                format!("{:#?}", r.response)
                    .replace(&format!(":{sink_port}"), ":PORT")
                    .replace(&format!(": {sink_port}"), ": PORT"),
            ),
            _ => None,
        })
        .collect();
    eprintln!("{transient:#?}");
    k9::snapshot!(
        &transient,
        r#"
[
    "Response {
    code: 400,
    enhanced_code: None,
    content: "KumoMTA internal: failed to connect to any candidate hosts: All failures are related to having an unplumbed source address. Are the network interfaces provisioned correctly? connect to 127.0.0.1:PORT and read initial banner: bind 9.9.9.9 for default failed: Cannot assign requested address (os error 99) while attempting to connect to 127.0.0.1:PORT transport=127.0.0.1:PORT proto=None",
    command: None,
}",
]
"#
    );

    Ok(())
}
