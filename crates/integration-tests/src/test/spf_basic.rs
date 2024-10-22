use crate::kumod::{generate_message_text, KumoDaemon, MailGenParams};
use rfc5321::{ClientError, Response};
use std::time::Duration;

#[tokio::test]
async fn spf_basic() -> anyhow::Result<()> {
    let mut daemon = KumoDaemon::spawn_with_policy("spf-basic.lua").await?;
    let mut client = daemon.smtp_client("localhost").await?;
    let body = generate_message_text(1024, 78);

    // Send mail from `localhost`, which is allowed

    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    // Send mail from `denied.localhost`, which is denied

    match daemon.smtp_client("denied.localhost").await {
        Ok(_) => panic!("expected rejection"),
        Err(err) => match err.downcast_ref::<ClientError>() {
            Some(ClientError::Rejected(Response { code: 550, .. })) => {}
            _ => panic!("expected ClientError"),
        },
    }

    // Send email with a denied sender

    let response = MailGenParams {
        body: Some(&body),
        sender: Some("foo@allowed.localhost"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop().await?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs()?;
    k9::snapshot!(
        delivery_summary,
        "
{
    Reception: 2,
    Delivery: 2,
    Rejection: 1,
}
"
    );
    Ok(())
}
