use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use bstr::{BString, ByteSlice};
use flate2::read::GzDecoder;
use k9::assert_equal;
use mailparsing::DecodedBody;
use std::io::Read;
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

    let mut messages = daemon.extract_maildir_messages()?;

    assert_equal!(messages.len(), 1);

    let parsed = messages[0].parsed()?;

    let pmap = parsed.child_parts()[1]
        .headers()
        .content_disposition()
        .unwrap()
        .unwrap()
        .parameter_map();

    let filename = pmap.get(&BString::from("filename")).clone().unwrap();

    let DecodedBody::Binary(bytes) = parsed.child_parts()[1].body().unwrap() else {
        panic!("expected binary data")
    };

    let mut decoder = GzDecoder::new(&bytes[..]);
    let mut decoded_string = String::new();
    decoder.read_to_string(&mut decoded_string).unwrap();

    // Spot check that the content looks sufficiently like a dmarc report.
    // We don't do a full validation here because testing the report shape
    // is covered by tests in kumo-dmarc itself
    assert!(filename.contains_str(b"testorg!"));
    assert!(decoded_string.contains("<email>dmarc-feedback@example.com</email>"));
    assert!(decoded_string.contains("<disposition>Reject</disposition>"));

    Ok(())
}
