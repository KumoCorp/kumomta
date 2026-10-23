use crate::kumod::DaemonWithMaildir;
use k9::assert_equal;
use rfc5321::*;
use std::time::Duration;

/// Builds a message body with interior lines separated by bare LF. If treated
/// as a CRLF-delimited line, it exceeds the 998-byte limit while each
/// LF-delimited line stays well within it. The final line uses CRLF to frame the
/// DATA terminator (`\r\n.\r\n`) cleanly. The interior bare LFs are the case
/// under test.
fn bare_lf_body() -> Vec<u8> {
    let line = "a".repeat(40);
    let mut body = String::from("Subject: bare lf\r\n\r\n");
    for i in 0..50 {
        body.push_str(&line);
        if i + 1 == 50 {
            body.push_str("\r\n");
        } else {
            body.push('\n');
        }
    }
    body.into_bytes()
}

async fn send_bare_lf(client: &mut SmtpClient) -> Result<Response, ClientError> {
    client
        .send_mail(
            ReversePath::try_from("sender@example.com").unwrap(),
            ForwardPath::try_from("recip@example.com").unwrap(),
            &bare_lf_body(),
        )
        .await
}

/// The default `Deny` disposition splits lines on CRLF only. A bare-LF body
/// has no CRLF to split on, so it reads as one over-long line and is rejected.
#[tokio::test]
async fn bare_lf_rejected_by_default() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let err = send_bare_lf(&mut client).await.unwrap_err();
    let resp = match err {
        ClientError::Rejected(resp) => resp,
        other => panic!("unexpected error: {other:#}"),
    };
    assert_equal!(resp.code, 500);
    assert_equal!(resp.content, "line too long");

    daemon.stop_both().await?;
    Ok(())
}

/// Under `invalid_line_endings="Fix"` the same body is measured per bare-LF line,
/// accepted, and delivered.
#[tokio::test]
async fn bare_lf_accepted_with_fix() -> anyhow::Result<()> {
    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_INVALID_LINE_ENDINGS", "Fix")]).await?;
    let mut client = daemon.smtp_client().await?;

    let resp = send_bare_lf(&mut client).await?;
    assert_equal!(resp.code, 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;
    assert_equal!(daemon.extract_maildir_messages()?.len(), 1);

    daemon.stop_both().await?;
    Ok(())
}
