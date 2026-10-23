use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use mailparsing::DecodedBody;
use std::time::Duration;

#[tokio::test]
async fn end_to_end_quoted_localpart() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("\"info@\"@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    // The summary, on its own, does nothing, but does implicitly exercise
    // the http listener and metrics endpoints to ensure that something is
    // at least tickling those during the test run.
    let summary = daemon.source.kcli_text(["queue-summary"]).await?;
    println!("summary is {summary}");

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 1,
    delivered: 1,
}
"
    );

    let mut messages = daemon.extract_maildir_messages()?;

    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;
    println!("headers: {:?}", parsed.headers());

    assert!(parsed.headers().get_first("Received").is_some());
    assert!(parsed.headers().get_first("X-KumoRef").is_some());

    // These two headers are added to all MailGenParams generated mail
    assert!(parsed.headers().get_first("X-Test1").is_some());
    assert!(parsed.headers().get_first("X-Another").is_some());

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
                        local_part: "info@",
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
