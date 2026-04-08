use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use rfc5321::parser::Command;

#[tokio::test]
async fn no_ports_in_rcpt_domain() -> anyhow::Result<()> {
    let daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    client
        .send_command(&Command::MailFrom {
            address: "sender@example.com".try_into().unwrap(),
            parameters: vec![],
        })
        .await?;
    let resp = client
        .send_command(&Command::Unknown(
            "RCPT TO:<sender@example.com:2025>".into(),
        ))
        .await?;

    k9::snapshot!(
        resp,
        r#"
Response {
    code: 501,
    enhanced_code: Some(
        EnhancedStatusCode {
            class: 5,
            subject: 1,
            detail: 3,
        },
    ),
    content: "Invalid recipient address syntax",
    command: Some(
        "RCPT TO:<sender@example.com:2025>\r
",
    ),
}
"#
    );

    Ok(())
}
