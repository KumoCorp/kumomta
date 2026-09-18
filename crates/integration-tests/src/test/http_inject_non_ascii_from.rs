use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

/// Regression test: a non-ASCII local-part in content.from/content.reply_to
/// used to get double-encoded (RFC 2047 encoded-word wrapped around the
/// addr-spec), producing a header that failed to parse back at all.
#[tokio::test]
async fn http_inject_non_ascii_from() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{
            "email": "user@example.com",
            "name": "Test User"
        }],
        "content": {
            "subject": "Non-ASCII From regression test",
            "text_body": "Hello {{ name }}!",
            "from": {
                "name": "Test",
                "email": "公式オンラインストア@example.com"
            },
            "reply_to": {
                "name": "Support",
                "email": "サポート@example.com"
            }
        }
    });

    let body = daemon.source.api_client().inject_v1(&payload).await?;
    assert_equal!(body.success_count, 1);
    assert_equal!(body.fail_count, 0);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;

    // Before the fix, from()/reply_to() would return an Err here.
    k9::snapshot!(
        parsed.headers().from(),
        r#"
Ok(
    Some(
        MailboxList(
            [
                Mailbox {
                    name: Some(
                        "Test",
                    ),
                    address: AddrSpec {
                        local_part: "公式オンラインストア",
                        domain: "example.com",
                    },
                },
            ],
        ),
    ),
)
"#
    );

    k9::snapshot!(
        parsed.headers().reply_to(),
        r#"
Ok(
    Some(
        AddressList(
            [
                Mailbox(
                    Mailbox {
                        name: Some(
                            "Support",
                        ),
                        address: AddrSpec {
                            local_part: "サポート",
                            domain: "example.com",
                        },
                    },
                ),
            ],
        ),
    ),
)
"#
    );

    Ok(())
}
