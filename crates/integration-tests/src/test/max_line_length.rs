use crate::kumod::{DaemonWithMaildir, MailGenParams};
use k9::assert_equal;
use rfc5321::*;

/// test maximum line length for a single SMTP command
#[tokio::test]
async fn max_line_length_noop() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    for n in [
        128, 512, 990, 991, 992, 993, 994, 995, 996, 997, 998, 999, 1000, 1001, 1024, 2048, 4192,
        1_000_000, 10_000_000,
    ] {
        let body = "a".repeat(n);
        let command = Command::Noop(Some(body));
        let command_text = command.encode();
        let command_len = command_text.len() - 2 /* CRLF */;
        eprintln!("command length: {command_len}");

        let response = client.send_command(&command).await?;
        if command_len > 998 {
            assert_equal!(
                response.code,
                500,
                "input length {command_len} (n={n}) should have failed"
            );
            assert_equal!(
                response.enhanced_code,
                Some(EnhancedStatusCode {
                    class: 5,
                    subject: 2,
                    detail: 3
                }),
                "input length {n}"
            );
            assert_equal!(
                response.content,
                "line too long",
                "input length {command_len}"
            );
        } else {
            assert_equal!(response.code, 250, "input length {command_len}");
        }
    }
    daemon.stop_both().await?;
    Ok(())
}

/// test maximum line length within DATA payload
#[tokio::test]
async fn max_line_length_in_data() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    for n in [
        128, 512, 990, 991, 992, 993, 994, 995, 996, 997, 998, 999, 1000, 1001, 1024, 2048, 4192,
        1_000_000, 10_000_000,
    ] {
        let nonsense = "a".repeat(n);
        let body = format!("Subject: length {n}\r\n\r\n{nonsense}\r\n",);

        let result = MailGenParams {
            full_content: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await;

        let response = match result {
            Ok(resp) => resp,
            Err(err) => {
                if let Some(ce) = err.downcast_ref::<ClientError>() {
                    match ce {
                        ClientError::Rejected(resp) => resp.clone(),
                        err => panic!("Unexpected failure for length {n}: {err:#}"),
                    }
                } else {
                    panic!("Unexpected failure for length {n}: {err:#}")
                }
            }
        };
        eprintln!("{response:?}");

        if n > 998 {
            assert_equal!(response.code, 500, "input length {n} should have failed");
            assert_equal!(
                response.enhanced_code,
                Some(EnhancedStatusCode {
                    class: 5,
                    subject: 2,
                    detail: 3
                }),
                "input length {n}"
            );
            assert_equal!(response.content, "line too long", "input length {n}");
        } else {
            assert_equal!(
                response.code,
                250,
                "input length {n} expected to be accepted"
            );
        }
    }
    daemon.stop_both().await?;
    Ok(())
}
