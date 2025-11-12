use crate::kumod::{generate_message_text, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_api_types::TraceSmtpV1Payload::Callback;
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
