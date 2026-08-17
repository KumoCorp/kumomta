use crate::kumod::{DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::RecordType;
use std::time::Duration;

/// A long supplemental `X-KumoRef` trace header survives relay to a strict SMTP
/// receiver, and its injected metadata is recovered in the feedback log when the
/// message is later reported back as an ARF abuse report.
#[tokio::test]
async fn end_to_end_long_trace_header() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start()
        .await
        .context("DaemonWithMaildir::start")?;

    // A per-recipient personalization value comparable to the real-world
    // payload that first exposed this issue. Base64-encoded inside the
    // X-KumoRef JSON, its single-line form exceeds the SMTP line length limit,
    // so relay is only possible once the header is folded.
    let token = "A".repeat(786);

    let payload = serde_json::json!({
        "envelope_sender": "sender@example.com",
        "recipients": [{
            "email": "recip@example.com",
            "metadata": {
                "token": token,
            }
        }],
        "trace_headers": {
            "supplemental_header": true,
            "include_meta_names": ["extra"],
        },
        "content": {
            "subject": "Long Trace Header Test",
            "text_body": "Hello!"
        }
    });

    let body = daemon.source.api_client().inject_v1(&payload).await?;
    assert_equal!(body.success_count, 1);
    assert_equal!(body.fail_count, 0);

    // The sink rejects any DATA line longer than the limit, so delivery
    // to it is only possible because the header was folded.
    let delivered = daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;
    daemon.dump_logs().await.context("dump_logs")?;
    anyhow::ensure!(
        delivered,
        "message with a long supplemental trace header was not delivered"
    );

    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let relayed = String::from_utf8(messages[0].read_data()?.to_vec())?;

    // Report the delivered message as abuse. The ARF embeds it verbatim,
    // folded X-KumoRef and all; on reception the source parses the report and
    // decodes the trace header into the feedback log record.
    let report = format!(
        concat!(
            "From: <abusedesk@example.com>\r\n",
            "To: <abuse@example.com>\r\n",
            "Subject: abuse report\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/report; report-type=feedback-report;\r\n",
            "    boundary=\"report-boundary\"\r\n",
            "\r\n",
            "--report-boundary\r\n",
            "Content-Type: message/feedback-report\r\n",
            "\r\n",
            "Feedback-Type: abuse\r\n",
            "User-Agent: SomeGenerator/1.0\r\n",
            "Version: 1\r\n",
            "\r\n",
            "--report-boundary\r\n",
            "Content-Type: message/rfc822\r\n",
            "\r\n",
            "{relayed}\r\n",
            "--report-boundary--\r\n",
        ),
        relayed = relayed,
    );

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams {
        full_content: Some(&report),
        sender: Some("abusedesk@example.com"),
        recip: Some("abuse@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send abuse report")?;
    anyhow::ensure!(
        response.code == 250,
        "abuse report was not accepted: {response:?}"
    );

    let logged = daemon
        .wait_for_source_summary(
            |summary| summary.get(&RecordType::Feedback).copied().unwrap_or(0) >= 1,
            Duration::from_secs(10),
        )
        .await;
    anyhow::ensure!(logged, "no feedback log record was written for the report");

    let records = daemon.source.collect_logs().await?;
    let feedback = records
        .iter()
        .find(|record| record.kind == RecordType::Feedback)
        .context("no feedback log record")?;
    let report = feedback
        .feedback_report
        .as_ref()
        .context("feedback log record has no report")?;
    assert_equal!(
        report.supplemental_trace,
        Some(serde_json::json!({
            "recipient": "recip@example.com",
            "extra": { "token": token },
        }))
    );

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}
