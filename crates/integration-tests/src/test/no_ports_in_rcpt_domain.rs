use crate::kumod::{DaemonWithMaildir, MailGenParams};
use anyhow::Context;

#[tokio::test]
async fn no_ports_in_rcpt_domain() -> anyhow::Result<()> {
    let daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let err = MailGenParams {
        recip: Some("someone@example.com:2025"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .unwrap_err();
    k9::snapshot!(
        err,
        r#"
Rejected(
    Response {
        code: 501,
        enhanced_code: None,
        content: "Syntax error in command or arguments:  --> 1:29
  |
1 | RCPT TO:<someone@example.com:2025>
  |                             ^---
  |
  = expected alpha, digit, or utf8_non_ascii",
        command: Some(
            "RCPT TO:<someone@example.com:2025>\r
",
        ),
    },
)
"#
    );

    Ok(())
}
