use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use kumo_api_types::TraceSmtpV1Payload;
use kumo_log_types::RecordType::Delivery;
use serde_json::json;
use std::time::Duration;

fn json_string(v: serde_json::Value) -> String {
    serde_json::to_string(&v).unwrap()
}

#[tokio::test]
async fn log_oob_arf() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        (
            "KUMOD_LISTENER_DOMAIN_MAP",
            &json_string(json!({
                "example.com": {
                    "relay_to": false,
                    "log_oob": true,
                    "log_arf": "LogThenRelay",
                },
                "thendrop.example.com": {
                    "relay_to": false,
                    "log_oob": "LogThenDrop",
                    "log_arf": "LogThenDrop",
                },

            })),
        ),
        ("KUMOD_RELAY_HOSTS", &json_string(json!([]))),
    ])
    .await
    .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let oob = include_str!("../../../kumo-log-types/data/rfc3464/1.eml");
    let arf = include_str!("../../../kumo-log-types/data/rfc5965/1.eml");

    let tracer = daemon.trace_server().await?;

    for (data, recip) in [
        (oob, "oob@example.com"),
        (oob, "oob@thendrop.example.com"),
        (arf, "arf@example.com"),
        (arf, "arf@thendrop.example.com"),
    ] {
        let response = MailGenParams {
            full_content: Some(data),
            recip: Some(recip),
            ..Default::default()
        }
        .send(&mut client)
        .await
        .context("send message")?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);
    }

    {
        // and verify that relaying for !report is not allowed
        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await
        .unwrap_err();
        k9::snapshot!(
            response,
            r#"
Rejected(
    Response {
        code: 550,
        enhanced_code: Some(
            EnhancedStatusCode {
                class: 5,
                subject: 7,
                detail: 1,
            },
        ),
        content: "relaying not permitted",
        command: Some(
            ".\r
",
        ),
    },
)
"#
        );
    }

    // Confirm the relaying disposition matches expectations
    let trace_events = tracer.stop().await?;
    #[derive(Debug)]
    #[allow(unused)]
    struct RelayDisp {
        was_arf_or_oob: bool,
        will_enqueue: bool,
        recipient: String,
    }

    let trace_dispositions: Vec<_> = trace_events
        .into_iter()
        .filter_map(|event| match event.payload {
            TraceSmtpV1Payload::MessageDisposition {
                was_arf_or_oob: Some(was_arf_or_oob),
                will_enqueue: Some(will_enqueue),
                recipient,
                ..
            } => Some(RelayDisp {
                was_arf_or_oob,
                will_enqueue,
                recipient,
            }),
            _ => None,
        })
        .collect();

    k9::snapshot!(
        trace_dispositions,
        r#"
[
    RelayDisp {
        was_arf_or_oob: true,
        will_enqueue: true,
        recipient: "oob@example.com",
    },
    RelayDisp {
        was_arf_or_oob: true,
        will_enqueue: false,
        recipient: "oob@thendrop.example.com",
    },
    RelayDisp {
        was_arf_or_oob: true,
        will_enqueue: true,
        recipient: "arf@example.com",
    },
    RelayDisp {
        was_arf_or_oob: true,
        will_enqueue: false,
        recipient: "arf@thendrop.example.com",
    },
    RelayDisp {
        was_arf_or_oob: false,
        will_enqueue: false,
        recipient: "recip@example.com",
    },
]
"#
    );

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 2,
            Duration::from_secs(10),
        )
        .await;

    daemon
        .wait_for_maildir_count(2, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;

    // 3 Receptions: 1 for the non-report, 2 for the LogThenRelay
    // 2 Deliveries: LogThenRelay
    // 2 OOB: because 2 oobs were received
    // 2 Feedback: because 2 arfs were received
    // 1 Rejection: for non-report because relay=false
    //
    // Sink receives and delivers 2 for the LogThenRelay messages.
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 3,
        Delivery: 2,
        OOB: 2,
        Feedback: 2,
        Rejection: 1,
    },
    sink_counts: {
        Reception: 2,
        Delivery: 2,
    },
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 3,
    delivered: 2,
}
"
    );

    assert_eq!(
        daemon.sink.maildir().count_new(),
        2,
        "sink.maildir: LogThenRelay == 2"
    );

    Ok(())
}
