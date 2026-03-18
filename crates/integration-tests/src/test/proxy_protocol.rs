use crate::kumod::{generate_message_text, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_api_types::TraceSmtpV1Payload::Callback;
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

#[tokio::test]
async fn proxy_protocol_switch_from_addr() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("proxy-source.lua")
        .env("KUMOD_TEST_REQUIRE_PROXY_PROTOCOL", "1")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let tracer: crate::kumod::ServerTracer = daemon.trace_sink().await?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    tracer
        .wait_for(
            |events| {
                events.iter().any(|event| {
                    matches!(&event.payload, Callback{name,..}
                        if name == "smtp_server_get_dynamic_parameters")
                })
            },
            Duration::from_secs(10),
        )
        .await;

    let trace_events = tracer.stop().await?;
    eprintln!("{trace_events:#?}");

    let re_evaluated_params = trace_events.iter().any(|event| {
        matches!(&event.payload, Callback{name,..}
            if name == "smtp_server_get_dynamic_parameters")
    });
    assert!(re_evaluated_params);

    let final_meta = &trace_events.last().unwrap().conn_meta;
    eprintln!("final_meta: {final_meta:#?}\n");

    assert!(final_meta.get("orig_received_from").is_some());
    assert!(final_meta.get("orig_received_via").is_some());

    k9::assert_equal!(
        final_meta.get("received_from").unwrap().to_string(),
        "\"127.0.0.1:0\""
    );
    k9::assert_equal!(
        final_meta.get("received_via").unwrap().to_string(),
        format!("\"{}\"", daemon.sink.listener("smtp").to_string())
    );

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}

/// This test just captures what happens if the proxy is unreachable.
/// The error message is likely linux-specific
#[tokio::test]
#[cfg(target_os = "linux")]
async fn proxy_protocol_broken_proxy() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("broken-proxy-source.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    let sink_port = daemon.sink.listener("smtp").port();

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) >= 1,
            Duration::from_secs(5),
        )
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    let transient: Vec<String> = logs
        .into_iter()
        .filter_map(|r| match r.kind {
            TransientFailure => Some(
                format!("{:#?}", r.response)
                    .replace(&format!(":{sink_port}"), ":PORT")
                    .replace(&format!(": {sink_port}"), ": PORT"),
            ),
            _ => None,
        })
        .collect();
    eprintln!("{transient:#?}");
    k9::snapshot!(
        &transient,
        r#"
[
    "Response {
    code: 400,
    enhanced_code: None,
    content: "KumoMTA internal: failed to connect to any candidate hosts: connect to 127.0.0.1:PORT and read initial banner: failed to connect to 255.255.255.255:1 HA { server: 255.255.255.255:1, addresses: IPv4(IPv4 { source_address: 127.0.0.1, source_port: 0, destination_address: 127.0.0.1, destination_port: PORT }), source: 127.0.0.1 }: Network is unreachable (os error 101)",
    command: None,
}",
]
"#
    );

    Ok(())
}
