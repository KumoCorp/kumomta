use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use anyhow::Context;
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
                    "log_arf": true,
                }

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
    for data in [oob, arf] {
        let response = MailGenParams {
            full_content: Some(data),
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

    daemon
        .wait_for_maildir_count(2, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 2,
        OOB: 1,
        Feedback: 1,
        Rejection: 1,
    },
    sink_counts: {},
}
"
    );
    k9::snapshot!(
        daemon.source.accounting_stats()?,
        "
AccountingStats {
    received: 2,
    delivered: 0,
}
"
    );

    Ok(())
}
