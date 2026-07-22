use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

/// Verify that what we send in transits through and is delivered
/// into the maildir at the other end with the same content
#[tokio::test]
async fn queue_dmarc() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("dmarc.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        recip: Some("permfail@sub.example.com"),
        body: Some("woot"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let mut messages = daemon.extract_maildir_messages()?;

    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;
    eprintln!("headers: {:?}", parsed.headers());

    assert!(parsed.headers().get_first("Received").is_some());
    assert!(parsed.headers().get_first("X-KumoRef").is_some());

    assert_equal!(parsed.headers().subject().unwrap().unwrap(), "DMARC Report");

    let body = parsed.body().unwrap();
    let body = body.to_string_lossy();

    assert!(body.contains("<email>dmarc-feedback@example.com</email>"));

    assert!(body.contains("<disposition>Reject</disposition>"));

    Ok(())
}
