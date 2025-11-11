use crate::kumod::{generate_message_text, KumoDaemon, MailGenParams};
use rfc5321::{ClientError, Response};
use std::time::Duration;

#[tokio::test]
async fn spf_basic() -> anyhow::Result<()> {
    let mut daemon = KumoDaemon::spawn_with_policy("spf-basic.lua").await?;
    let mut client = daemon.smtp_client("localhost").await?;
    let body = generate_message_text(1024, 78);

    // Send mail from `localhost`, which is allowed through `smtp_server_ehlo`
    // but gets a temporary error from `smtp_server_message_received` because
    // there no SPF policy is found for the sender's domain, `example.com`.

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
            Some(ClientError::Rejected(Response {
                code: 550, content, ..
            })) => {
                assert_eq!(content, "SPF EHLO check failed helo=denied.localhost");
            }
            _ => panic!("expected ClientError"),
        },
    }

    // Send email with an allowed sender

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

    let delivery_summary = daemon.dump_logs().await?;
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

    let mut entries = daemon.maildir().list_new().collect::<Result<Vec<_>, _>>()?;
    assert_eq!(entries.len(), 2);
    for entry in &mut entries {
        let headers = entry.headers()?;
        let from = headers.from().unwrap().unwrap().0;
        let results = headers.authentication_results().unwrap().unwrap().results;
        eprintln!("{results:#?}");
        match from[0].address.domain.as_str() {
            "example.com" => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].result, "none");
                assert_eq!(results[0].method, "spf");
                assert_eq!(
                    results[0].reason.as_deref().unwrap(),
                    "no SPF records found for example.com"
                );
                assert_eq!(
                    results[0].props.get("smtp.mailfrom").unwrap(),
                    "sender@example.com"
                );
            }
            "allowed.localhost" => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].result, "pass");
                assert_eq!(results[0].method, "spf");
                assert_eq!(
                    results[0].reason.as_deref().unwrap(),
                    "matched 'all' directive"
                );
                assert_eq!(
                    results[0].props.get("smtp.mailfrom").unwrap(),
                    "foo@allowed.localhost"
                );
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}
