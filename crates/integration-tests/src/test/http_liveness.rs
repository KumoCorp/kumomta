use crate::kumod::{DaemonWithMaildir, DaemonWithTsa};
use anyhow::Context;

#[tokio::test]
async fn http_liveness_kumod() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;

    let client = reqwest::Client::new();
    let response = client
        .get(&format!(
            "http://{}/api/check-liveness/v1",
            daemon.sink.listener("http")
        ))
        .send()
        .await?;

    let status = response.status();

    let body_bytes = response
        .text()
        .await
        .context("failed to read error response body")?;

    k9::assert_equal!(format!("{status} {body_bytes}"), "200 OK OK");

    daemon.stop_both().await?;

    Ok(())
}

#[tokio::test]
async fn http_liveness_tsa() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let client = reqwest::Client::new();
    let response = client
        .get(&format!(
            "http://{}/tsa/status",
            daemon.tsa.listener("http")
        ))
        .send()
        .await?;

    let status = response.status();

    let body_bytes = response
        .text()
        .await
        .context("failed to read error response body")?;

    k9::assert_equal!(format!("{status} {body_bytes}"), "200 OK TSA Daemon OK");

    daemon.stop().await?;

    Ok(())
}
