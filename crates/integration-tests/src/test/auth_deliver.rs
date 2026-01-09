use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use std::time::Duration;

#[tokio::test]
async fn auth_deliver() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_SMTP_AUTH_USERNAME", "daniel"),
        ("KUMOD_SMTP_AUTH_PASSWORD", "tiger"),
    ])
    .await?;

    let mut client = daemon.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );
    daemon.assert_no_acct_deny().await?;
    Ok(())
}
