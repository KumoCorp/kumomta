use crate::kumod::DaemonWithMaildir;
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

/// Regression test for the From/Reply-To header corruption bug.
///
/// `content.from`/`content.reply_to` are built correctly, address-aware, via
/// `Mailbox::encode_value()` in `normalize()`. Before the fix, that string was
/// then flattened into the generic `headers: BTreeMap<String, String>` bag and
/// re-encoded wholesale by `Header::new_unstructured`'s blanket `qp_encode` in
/// `expand_for_recip()`, which wraps the *entire* value -- including the
/// addr-spec -- inside a single RFC 2047 encoded-word. That's illegal per
/// RFC 2047 §5 rule 3 ("An encoded-word MUST NOT appear within an
/// addr-spec") and produced a header that fails to parse back at all.
///
/// See KUMO_DISCORD_FROM_ADDRESS_BUG_WALKTHROUGH.md for the full trace.
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

    // The actual regression check: before the fix, `from()`/`reply_to()`
    // returned an `Err` here (the nom address-list grammar can't parse an
    // RFC 2047 encoded-word straddling the addr-spec). After the fix it must
    // parse cleanly and preserve the exact, unmangled UTF-8 address and
    // display name.
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
