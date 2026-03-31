use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use rfc5321::Command;

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
        .send_command(&Command::RawLine(
            "RCPT TO:<sender@example.com:2025>".into(),
        ))
        .await?;

    k9::snapshot!(
        resp,
        r#"
Response {
    code: 501,
    enhanced_code: None,
    content: "Syntax error in command or arguments:  --> 1:28
  |
1 | RCPT TO:<sender@example.com:2025>
  |                            ^---
  |
  = expected alpha, digit, or utf8_non_ascii",
    command: Some(
        "RCPT TO:<sender@example.com:2025>\r
",
    ),
}
"#
    );

    Ok(())
}
