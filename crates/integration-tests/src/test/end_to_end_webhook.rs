use crate::kumod::{generate_message_text, DaemonWithMaildirAndWebHook, MailGenParams};
use k9::assert_equal;
use kumo_log_types::RecordType;
use mailparsing::DecodedBody;
use std::collections::BTreeMap;
use std::time::Duration;

/// Verify that what we send in transits through and is delivered
/// into the maildir at the other end with the same content,
/// and that the webhook logging is also used and captures
/// the correct amount of records
#[tokio::test]
async fn end_to_end_with_webhook() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirAndWebHook::start().await?;

    eprintln!("sending message");
    let mut client = daemon.with_maildir.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .with_maildir
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon
        .wait_for_webhook_record_count(2, Duration::from_secs(10))
        .await;

    daemon.stop().await?;
    println!("Stopped!");

    let delivery_summary = daemon.with_maildir.dump_logs()?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 3,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );

    let webhook_summary = daemon.webhook.dump_logs();
    k9::snapshot!(
        webhook_summary,
        "
Ok(
    {
        Reception: 1,
        Delivery: 1,
    },
)
"
    );

    let mut logged_headers = vec![];
    let mut headers_delivered = vec![];
    for record in daemon.webhook.return_logs() {
        let ordered_headers: BTreeMap<_, _> = record.headers.into_iter().collect();
        if record.kind == RecordType::Reception {
            logged_headers.push(ordered_headers);
        } else if record.kind == RecordType::Delivery {
            headers_delivered.push(ordered_headers);
        }
    }
    k9::snapshot!(
        logged_headers,
        r#"
[
    {
        "Subject": String("Hello! This is a test"),
        "X-Another": String("Another"),
        "X-KumoRef": String("eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20ifQ=="),
        "X-Test1": String("Test1"),
    },
]
"#
    );
    k9::snapshot!(
        headers_delivered,
        r#"
[
    {
        "Subject": String("Hello! This is a test"),
        "X-Another": String("Another"),
        "X-KumoRef": String("eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20ifQ=="),
        "X-Test1": String("Test1"),
    },
]
"#
    );

    let mut messages = daemon.with_maildir.extract_maildir_messages()?;

    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;
    println!("headers: {:?}", parsed.headers());

    assert!(parsed.headers().get_first("Received").is_some());
    assert!(parsed.headers().get_first("X-KumoRef").is_some());
    k9::snapshot!(
        parsed.headers().from().unwrap(),
        r#"
Some(
    MailboxList(
        [
            Mailbox {
                name: None,
                address: AddrSpec {
                    local_part: "sender",
                    domain: "example.com",
                },
            },
        ],
    ),
)
"#
    );
    k9::snapshot!(
        parsed.headers().to().unwrap(),
        r#"
Some(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: None,
                    address: AddrSpec {
                        local_part: "recip",
                        domain: "example.com",
                    },
                },
            ),
        ],
    ),
)
"#
    );
    assert_equal!(
        parsed.headers().subject().unwrap().unwrap(),
        "Hello! This is a test"
    );
    assert_equal!(parsed.body().unwrap(), DecodedBody::Text(body.into()));

    Ok(())
}
