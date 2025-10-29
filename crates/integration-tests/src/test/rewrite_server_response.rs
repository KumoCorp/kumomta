use crate::kumod::{generate_message_text, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;

/// Verify that what we send in transits through and is delivered
/// into the maildir at the other end with the same content
#[tokio::test]
async fn rewrite_data_response() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("rewrite-ok.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    daemon.stop_both().await.context("stop_both")?;

    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);
    anyhow::ensure!(
        response.content.starts_with("super fantastic!"),
        response.content
    );

    Ok(())
}
