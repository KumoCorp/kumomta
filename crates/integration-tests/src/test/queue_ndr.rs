use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::rfc3464::Report;
use kumo_log_types::RecordType;
use kumo_log_types::RecordType::{Bounce, Reception};
use std::time::Duration;

/// Verify that what we send in transits through and is delivered
/// into the maildir at the other end with the same content
#[tokio::test]
async fn queue_ndr() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("ndr.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        recip: Some("permfail@example.com"),
        body: Some("woot"),
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
        Bounce: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
        Rejection: 2,
    },
}
"
    );

    let mut messages = daemon.extract_maildir_messages()?;

    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;
    eprintln!("headers: {:?}", parsed.headers());

    assert!(parsed.headers().get_first("Received").is_some());
    assert!(parsed.headers().get_first("X-KumoRef").is_some());

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
                        local_part: "sender",
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
        "Returned mail"
    );

    eprintln!("{}", String::from_utf8_lossy(messages[0].read_data()?));
    let mut report = Report::parse(messages[0].read_data()?)
        .expect("failed to parse report")
        .expect("expected a report");

    // A little dance because the Received header has a uuid that
    // we can't simply do static snapshot matching on
    let original_message = report
        .original_message
        .take()
        .expect("DSN to include original message");

    report.per_message.arrival_date.take().unwrap();
    for r in report.per_recipient.iter_mut() {
        r.last_attempt_date.take().unwrap();
    }

    k9::snapshot!(
        &report,
        r#"
Report {
    per_message: PerMessageReportEntry {
        original_envelope_id: None,
        reporting_mta: RemoteMta {
            mta_type: "dns",
            name: "mta1.example.com",
        },
        dsn_gateway: None,
        received_from_mta: None,
        arrival_date: None,
        extensions: {},
    },
    per_recipient: [
        PerRecipientReportEntry {
            final_recipient: Recipient {
                recipient_type: "rfc822",
                recipient: "permfail@example.com",
            },
            action: Failed,
            status: ReportStatus {
                class: 5,
                subject: 0,
                detail: 0,
                comment: Some(
                    "permfail requested",
                ),
            },
            original_recipient: None,
            remote_mta: Some(
                RemoteMta {
                    mta_type: "dns",
                    name: "localhost",
                },
            ),
            diagnostic_code: Some(
                DiagnosticCode {
                    diagnostic_type: "smtp",
                    diagnostic: "500 permfail requested",
                },
            ),
            last_attempt_date: None,
            final_log_id: None,
            will_retry_until: None,
            extensions: {},
        },
    ],
    original_message: None,
}
"#
    );

    assert!(original_message.contains("woot"));

    Ok(())
}

/// Validate behavior when a generated NDR itself bounces.
/// The expectation is that we log the Bounce for the generated
/// NDR but don't generate an NDR for the bounced NDR
#[tokio::test]
async fn ndr_bounces() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("ndr.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let response = MailGenParams {
        recip: Some("permfail@example.com"),
        sender: Some("sender-permfail@example.com"),
        body: Some("woot"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&Reception).copied().unwrap_or(0) > 0
                    && summary.get(&Bounce).copied().unwrap_or(0) >= 2
            },
            Duration::from_secs(10),
        )
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Bounce: 2,
    },
    sink_counts: {
        Rejection: 5,
    },
}
"
    );

    let records = daemon.source.collect_logs().await?;
    let mut bounces = vec![];
    for record in records {
        if record.kind == RecordType::Bounce {
            bounces.push(format!(
                "from=<{}> to=<{}> why='{}' subject={}",
                record.sender,
                record.recipient,
                record.response.content,
                record.headers.get("Subject").unwrap().to_string(),
            ));
        }
    }

    k9::snapshot!(
        bounces,
        r#"
[
    "from=<sender-permfail@example.com> to=<permfail@example.com> why='permfail requested' subject="Hello! This is a test"",
    "from=<> to=<sender-permfail@example.com> why='permfail requested' subject="Returned mail"",
]
"#
    );

    Ok(())
}
