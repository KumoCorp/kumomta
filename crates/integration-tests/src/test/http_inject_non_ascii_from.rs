use crate::kumod::DaemonWithMaildirOptions;
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

/// Regression test: a non-ASCII local-part in content.from/content.reply_to
/// used to get double-encoded (RFC 2047 encoded-word wrapped around the
/// addr-spec), producing a header that failed to parse back at all.
///
/// Uses a custom source policy (source-verify-from-header.lua) that calls
/// the real msg:from_header()/msg:get_address_header("Reply-To") Lua
/// bindings in an http_message_generated hook, so this exercises the actual
/// production code path, not just a Rust-side re-check of the same parse.
#[tokio::test]
async fn http_inject_non_ascii_from() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-verify-from-header.lua")
        .start()
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

    // Matches Message::get_address_header()'s hdr.as_address_list() call,
    // which is what the Lua-exposed msg:from_header() actually runs in
    // production -- HeaderMap::from() uses as_mailbox_list() instead, a
    // different top-level grammar rule, so it wouldn't prove this path.
    // Before the fix, this would return an Err here.
    k9::snapshot!(
        parsed
            .headers()
            .get_first("From")
            .expect("From header present")
            .as_address_list(),
        r#"
Ok(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: Some(
                        "Test",
                    ),
                    address: AddrSpec {
                        local_part: "公式オンラインストア",
                        domain: "example.com",
                    },
                },
            ),
        ],
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
