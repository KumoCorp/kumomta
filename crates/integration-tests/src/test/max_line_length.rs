use crate::kumod::{generate_nonsense_string, DaemonWithMaildir};
use k9::assert_equal;
use rfc5321::*;

/// test maximum line length
#[tokio::test]
async fn max_line_length() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    for n in [128, 512, 1024, 2048, 4192, 1_000_000, 10_000_000] {
        eprintln!("Body length: {n}");
        let body = generate_nonsense_string(n);
        let response = client.send_command(&Command::Noop(Some(body))).await?;
        if n > 998 {
            assert_equal!(response.code, 500);
            assert_equal!(
                response.enhanced_code,
                Some(EnhancedStatusCode {
                    class: 5,
                    subject: 2,
                    detail: 3
                })
            );
            assert_equal!(response.content, "line too long");
        } else {
            assert_equal!(response.code, 250);
        }
    }
    daemon.stop_both().await?;
    Ok(())
}
