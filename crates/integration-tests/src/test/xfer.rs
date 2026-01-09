use crate::kumod::{DaemonWithMaildir, MailGenParams};
use kumo_api_types::{InspectQueueV1Response, SuspendV1Response};
use kumo_log_types::RecordType;
use kumo_log_types::RecordType::{Delivery, Reception, TransientFailure, XferIn, XferOut};
use mailparsing::MimePart;
use std::time::Duration;

/// This test suspends delivery to example.com for tenant "mytenant",
/// causing mail to be delayed for ~20 minutes.
/// Then it issues an xfer for example.com (all tenants) to move it directly
/// to the sink, where it should retain its delayed status.
#[tokio::test]
async fn xfer_end_to_end() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let status: SuspendV1Response = daemon
        .kcli_json([
            "suspend",
            "--domain",
            "example.com",
            "--tenant",
            "mytenant",
            "--reason",
            "smtp example.com suspended",
        ])
        .await?;
    println!("kcli status: {status:?}");

    let response = MailGenParams {
        recip: Some("allow@example.com"),
        full_content: Some(
            "X-Schedule: {\"expires\":\"3000-12-31T00:00:00Z\"}\r\n\
            Subject: Hello! This is a test\r\n\
            Tenant: mytenant\r\n\
            \r\n\
            Woot\r\n",
        ),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&Reception).copied().unwrap_or(0) > 0
                    && summary.get(&TransientFailure).copied().unwrap_or(0) > 0
            },
            Duration::from_secs(50),
        )
        .await;

    let status: InspectQueueV1Response = daemon
        .kcli_json(["inspect-sched-q", "--want-body", "mytenant@example.com"])
        .await?;
    eprintln!("{status:#?}");

    let message_before = &status.messages[0].message;

    // Transfer example.com -> sink
    daemon
        .kcli([
            "xfer",
            "--domain",
            "example.com",
            "--reason",
            "testing",
            "--target",
            &format!("http://{}", daemon.sink.listener("http")),
        ])
        .await?;

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&XferOut).copied().unwrap_or(0) > 0,
            Duration::from_secs(10),
        )
        .await;
    daemon
        .wait_for_sink_summary(
            |summary| summary.get(&XferIn).copied().unwrap_or(0) > 0,
            Duration::from_secs(5),
        )
        .await;

    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        TransientFailure: 1,
        AdminRebind: 1,
        XferOut: 1,
    },
    sink_counts: {
        Delayed: 1,
        XferIn: 1,
    },
}
"
    );

    // Our injected message should be delayed (TransientFailure) due
    // to the suspension, then rebound into an xfer queue by the xfer
    // request, before being transferred out
    let source_records: Vec<RecordType> = daemon
        .source
        .collect_logs()
        .await?
        .into_iter()
        .map(|r| r.kind)
        .collect();
    k9::snapshot!(
        source_records,
        "
[
    Reception,
    TransientFailure,
    AdminRebind,
    XferOut,
]
"
    );

    // The transferred message should retain its due time and show
    // as Delayed
    let sink_logs = daemon.sink.collect_logs().await?;
    let sink_records: Vec<RecordType> = sink_logs.iter().map(|r| r.kind).collect();
    k9::snapshot!(
        &sink_records,
        "
[
    XferIn,
    Delayed,
]
"
    );

    // Let's inspect the message
    let status: InspectQueueV1Response = daemon
        .sink_kcli_json(["inspect-sched-q", "--want-body", "mytenant@example.com"])
        .await?;
    eprintln!("{status:#?}");

    let message_after = &status.messages[0].message;

    daemon.stop_both().await?;

    k9::assert_equal!(message_before.due, message_after.due);
    k9::assert_equal!(message_before.data, message_after.data);
    k9::assert_equal!(message_before.sender, message_after.sender);
    k9::assert_equal!(message_before.recipient, message_after.recipient);
    k9::assert_equal!(message_before.num_attempts, message_after.num_attempts);

    assert!(
        message_after.scheduling.is_some(),
        "scheduling header was picked up and carried over"
    );
    k9::assert_equal!(message_before.scheduling, message_after.scheduling);

    for (k, v) in message_before.meta.as_object().unwrap().iter() {
        k9::assert_equal!(message_after.meta.get(k), Some(v));
    }

    k9::assert_equal!(
        message_after
            .meta
            .get("received_via")
            .unwrap()
            .as_str()
            .unwrap(),
        daemon.source.listener("smtp").to_string(),
        "received_via is copied across"
    );

    // This is coupled with maildir-sink.lua's xfer_message_received hook impl
    k9::assert_equal!(
        message_after.meta.get("sunk").unwrap().as_str().unwrap(),
        "received via xfer",
        "xfer_message_received event was triggered and mutated the message"
    );

    let msg = MimePart::parse(message_after.data.as_deref().unwrap()).unwrap();

    k9::assert_equal!(
        msg.headers().subject().unwrap().unwrap(),
        "Hello! This is a test"
    );

    Ok(())
}

/// This test verifies that it is possible to xfer an xfer to change its
/// destination to a different target
#[tokio::test]
async fn xfer_twice() -> anyhow::Result<()> {
    let daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let status: SuspendV1Response = daemon
        .kcli_json([
            "suspend",
            "--domain",
            "example.com",
            "--reason",
            "smtp example.com suspended",
        ])
        .await?;
    println!("kcli status: {status:?}");

    let status: SuspendV1Response = daemon
        .kcli_json([
            "suspend",
            "--queue",
            "http://bogus.kumomta.internal.notavalidtld/.xfer.kumomta.internal",
            "--reason",
            "bogus xfer suspended",
        ])
        .await?;
    println!("kcli status: {status:?}");

    let response = MailGenParams {
        recip: Some("allow@example.com"),
        full_content: Some(
            "X-Schedule: {\"expires\":\"3000-12-31T00:00:00Z\"}\r\n\
            Subject: Hello! This is a test\r\n\
            Tenant: mytenant\r\n\
            \r\n\
            Woot\r\n",
        ),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&Reception).copied().unwrap_or(0) > 0
                    && summary.get(&TransientFailure).copied().unwrap_or(0) > 0
            },
            Duration::from_secs(10),
        )
        .await;

    // Transfer example.com -> bogus suspended destination
    daemon
        .kcli([
            "xfer",
            "--domain",
            "example.com",
            "--reason",
            "testing",
            "--target",
            "http://bogus.kumomta.internal.notavalidtld",
        ])
        .await?;

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&Reception).copied().unwrap_or(0) > 0
                    && summary.get(&TransientFailure).copied().unwrap_or(0) > 1
            },
            Duration::from_secs(10),
        )
        .await;

    // Now transfer it to the sink
    daemon
        .kcli([
            "xfer",
            "--queue",
            "http://bogus.kumomta.internal.notavalidtld/.xfer.kumomta.internal",
            "--reason",
            "testing",
            "--target",
            &format!("http://{}", daemon.sink.listener("http")),
        ])
        .await?;

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&XferOut).copied().unwrap_or(0) > 0,
            Duration::from_secs(10),
        )
        .await;

    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        TransientFailure: 2,
        AdminRebind: 2,
        XferOut: 1,
    },
    sink_counts: {
        Delayed: 1,
        XferIn: 1,
    },
}
"
    );

    // Our injected message should be delayed (TransientFailure) due
    // to the suspension, then rebound into an xfer queue by the first
    // xfer request, which hits the other suspension, then rebound
    // again before being transferred out
    let http_endpoint = daemon.sink.listener("http").to_string();
    let source_records: Vec<(RecordType, String /* queue */)> = daemon
        .source
        .collect_logs()
        .await?
        .into_iter()
        .map(|r| (r.kind, r.queue.replace(&http_endpoint, "SINK")))
        .collect();
    k9::snapshot!(
        source_records,
        r#"
[
    (
        Reception,
        "mytenant@example.com",
    ),
    (
        TransientFailure,
        "mytenant@example.com",
    ),
    (
        AdminRebind,
        "http://bogus.kumomta.internal.notavalidtld/.xfer.kumomta.internal",
    ),
    (
        TransientFailure,
        "http://bogus.kumomta.internal.notavalidtld/.xfer.kumomta.internal",
    ),
    (
        AdminRebind,
        "http://SINK/.xfer.kumomta.internal",
    ),
    (
        XferOut,
        "http://SINK/.xfer.kumomta.internal",
    ),
]
"#
    );

    Ok(())
}

/// This test verifies that it is possible to cancel an xfer
#[tokio::test]
async fn xfer_cancel() -> anyhow::Result<()> {
    let daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let suspend_status: SuspendV1Response = daemon
        .kcli_json([
            "suspend",
            "--domain",
            "example.com",
            "--reason",
            "smtp example.com suspended",
        ])
        .await?;
    println!("kcli status: {suspend_status:?}");

    let status: SuspendV1Response = daemon
        .kcli_json([
            "suspend",
            "--queue",
            "http://bogus.kumomta.internal.notavalidtld/.xfer.kumomta.internal",
            "--reason",
            "bogus xfer suspended",
        ])
        .await?;
    println!("kcli status: {status:?}");

    let response = MailGenParams {
        recip: Some("allow@example.com"),
        full_content: Some(
            "X-Schedule: {\"expires\":\"3000-12-31T00:00:00Z\"}\r\n\
            Subject: Hello! This is a test\r\n\
            Tenant: mytenant\r\n\
            \r\n\
            Woot\r\n",
        ),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&Reception).copied().unwrap_or(0) > 0
                    && summary.get(&TransientFailure).copied().unwrap_or(0) > 0
            },
            Duration::from_secs(10),
        )
        .await;

    // Transfer example.com -> bogus suspended destination
    daemon
        .kcli([
            "xfer",
            "--domain",
            "example.com",
            "--reason",
            "testing",
            "--target",
            "http://bogus.kumomta.internal.notavalidtld",
        ])
        .await?;

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&Reception).copied().unwrap_or(0) > 0
                    && summary.get(&TransientFailure).copied().unwrap_or(0) > 1
            },
            Duration::from_secs(10),
        )
        .await;

    // Allow smtp to flow again
    daemon
        .kcli(["suspend-cancel", "--id", &suspend_status.id.to_string()])
        .await?;

    // Now to cancel the xfer
    daemon
        .kcli([
            "xfer-cancel",
            "http://bogus.kumomta.internal.notavalidtld/.xfer.kumomta.internal",
            "--reason",
            "put that thing back where it came from or so help me",
        ])
        .await?;

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
            Duration::from_secs(10),
        )
        .await;

    daemon.assert_no_acct_deny().await?;
    let delivery_summary = daemon.dump_logs().await?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
        TransientFailure: 2,
        AdminRebind: 2,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
    );

    // Our injected message should be delayed (TransientFailure) due
    // to the suspension, then rebound into an xfer queue by the first
    // xfer request, which hits the other suspension, then rebound
    // again before being transferred out
    let http_endpoint = daemon.sink.listener("http").to_string();
    let source_records: Vec<(RecordType, String /* queue */)> = daemon
        .source
        .collect_logs()
        .await?
        .into_iter()
        .map(|r| (r.kind, r.queue.replace(&http_endpoint, "SINK")))
        .collect();
    k9::snapshot!(
        source_records,
        r#"
[
    (
        Reception,
        "mytenant@example.com",
    ),
    (
        TransientFailure,
        "mytenant@example.com",
    ),
    (
        AdminRebind,
        "http://bogus.kumomta.internal.notavalidtld/.xfer.kumomta.internal",
    ),
    (
        TransientFailure,
        "http://bogus.kumomta.internal.notavalidtld/.xfer.kumomta.internal",
    ),
    (
        AdminRebind,
        "mytenant@example.com",
    ),
    (
        Delivery,
        "mytenant@example.com",
    ),
]
"#
    );

    Ok(())
}
