use crate::kumod::{generate_message_text, DaemonWithMaildirAndWebHook, MailGenParams};
use std::time::Duration;

/// Verify that what we send in transits through and is delivered
/// into the maildir at the other end with the same content,
/// and that the webhook logging is also used and captures
/// the correct amount of records
#[tokio::test]
async fn end_to_end_with_webhook_batch() -> anyhow::Result<()> {
    let batch_size = 10;
    let mut daemon = DaemonWithMaildirAndWebHook::start_batched(batch_size).await?;

    eprintln!("sending message");
    let mut client = daemon.with_maildir.smtp_client().await?;

    for _ in 0..batch_size {
        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);
    }

    daemon
        .with_maildir
        .wait_for_maildir_count(batch_size, Duration::from_secs(10))
        .await;

    daemon
        .wait_for_webhook_record_count(2 * batch_size, Duration::from_secs(10))
        .await;

    daemon.stop().await?;
    println!("Stopped!");

    let webhook_logs = daemon.webhook.return_logs();
    assert_eq!(webhook_logs.len(), 2 * batch_size);
    Ok(())
}
