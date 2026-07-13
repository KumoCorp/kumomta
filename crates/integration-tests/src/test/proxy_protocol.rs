use crate::kumod::{generate_message_text, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_api_types::TraceSmtpV1Payload::Callback;
use kumo_log_types::RecordType::Delivery;
#[cfg(target_os = "linux")]
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

/// Like `proxy_protocol_switch_from_addr` but the ha_proxy_server is
/// configured as a DNS host name rather than an IP literal. A test resolver
/// maps `proxy.test` to 127.0.0.1; the source kumod must resolve the name
/// at connect time and connect through the resolved address. The source's
/// Delivery log captures the actually-used proxy address, so asserting on
/// it proves resolution actually ran and produced the expected IP.
#[tokio::test]
async fn proxy_protocol_hostname() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("ha-proxy-source-hostname.lua")
        .env("KUMOD_TEST_REQUIRE_PROXY_PROTOCOL", "1")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let sink_port = daemon.sink.listener("smtp").port();

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    let deliveries: Vec<_> = logs.iter().filter(|r| r.kind == Delivery).collect();
    k9::assert_equal!(deliveries.len(), 1, "expected exactly one Delivery record");
    let sa = deliveries[0]
        .source_address
        .as_ref()
        .expect("Delivery record should carry a source_address");
    k9::assert_equal!(sa.protocol.as_deref(), Some("haproxy"));
    let server = sa.server.expect("haproxy delivery should record server");
    k9::assert_equal!(server.to_string(), format!("127.0.0.1:{sink_port}"));

    Ok(())
}

/// The ha_proxy_server hostname resolves to two A records: an unreachable
/// address (255.255.255.255) followed by a reachable one (127.0.0.1). The
/// first connect attempt fails with a network error; `connect_to` must
/// fall back to the second candidate and succeed.
///
/// We assert that the message was delivered through 127.0.0.1 (proving the
/// successful candidate is the second one) AND that the
/// `proxy_connection_failures` counter incremented (proving the first
/// candidate was actually tried, rather than 127.0.0.1 happening to be
/// the only attempt).
#[tokio::test]
#[cfg(target_os = "linux")]
async fn proxy_protocol_hostname_failover() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("ha-proxy-source-hostname-failover.lua")
        .env("KUMOD_TEST_REQUIRE_PROXY_PROTOCOL", "1")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let sink_port = daemon.sink.listener("smtp").port();

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(20))
        .await;

    // Confirm the per-candidate failure counter ticked at least once,
    // which can only happen if the first (unreachable) candidate was
    // actually attempted before falling back to the second.
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| m.name().as_str() == "proxy_connection_failures",
            |values| values.iter().any(|v| *v >= 1.0),
        )
        .await
        .context("proxy_connection_failures should be >= 1 after failover")?;

    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    let deliveries: Vec<_> = logs.iter().filter(|r| r.kind == Delivery).collect();
    k9::assert_equal!(deliveries.len(), 1, "expected exactly one Delivery record");
    let sa = deliveries[0]
        .source_address
        .as_ref()
        .expect("Delivery record should carry a source_address");
    k9::assert_equal!(sa.protocol.as_deref(), Some("haproxy"));
    let server = sa.server.expect("haproxy delivery should record server");
    // The successful candidate must be the second (reachable) one.
    k9::assert_equal!(server.to_string(), format!("127.0.0.1:{sink_port}"));

    Ok(())
}

/// All candidates for a hostname-form ha_proxy_server fail to connect.
/// Validates that `combine_connect_errors` aggregates per-candidate
/// failures into a single ConnectError whose message lists each
/// candidate and the reason it failed.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn proxy_protocol_hostname_all_fail() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("ha-proxy-source-hostname-all-fail.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let sink_port = daemon.sink.listener("smtp").port();

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) >= 1,
            Duration::from_secs(10),
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
    content: "KumoMTA internal: failed to connect to any candidate hosts: All failures are related to proxy connection issues. Is the proxy infrastructure online and healthy? connect to 127.0.0.1:PORT and read initial banner: failed to connect via any of 2 candidate(s) for source default: 255.255.255.255:1: failed to connect to 255.255.255.255:1 HA { server: 255.255.255.255:1, addresses: IPv4(IPv4 { source_address: 127.0.0.1, source_port: 0, destination_address: 127.0.0.1, destination_port: PORT }), source: 127.0.0.1 }: Network is unreachable (os error 101); 127.0.0.99:1: failed to connect to 127.0.0.99:1 HA { server: 127.0.0.99:1, addresses: IPv4(IPv4 { source_address: 127.0.0.1, source_port: 0, destination_address: 127.0.0.1, destination_port: PORT }), source: 127.0.0.1 }: Connection refused (os error 111)",
    command: None,
}",
]
"#
    );

    Ok(())
}

/// The ha_proxy_server hostname resolves only to a AAAA record but the
/// configured source_address is IPv4. The candidate filter must drop the
/// AAAA candidate before bind() is attempted and produce a clear error.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn proxy_protocol_hostname_family_mismatch() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("ha-proxy-source-hostname-family-mismatch.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let sink_port = daemon.sink.listener("smtp").port();

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) >= 1,
            Duration::from_secs(10),
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
    content: "KumoMTA internal: failed to connect to any candidate hosts: connect to 127.0.0.1:PORT and read initial banner: source default: no ha_proxy_server candidates remain after filtering 1 resolved address(es) to match source_address 127.0.0.1 family",
    command: None,
}",
]
"#
    );

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
    content: "KumoMTA internal: failed to connect to any candidate hosts: All failures are related to proxy connection issues. Is the proxy infrastructure online and healthy? connect to 127.0.0.1:PORT and read initial banner: failed to connect to 255.255.255.255:1 HA { server: 255.255.255.255:1, addresses: IPv4(IPv4 { source_address: 127.0.0.1, source_port: 0, destination_address: 127.0.0.1, destination_port: PORT }), source: 127.0.0.1 }: Network is unreachable (os error 101)",
    command: None,
}",
]
"#
    );

    Ok(())
}
