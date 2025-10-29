use crate::kumod::DaemonWithMaildirOptions;
use anyhow::Context;
use kumo_api_types::TraceSmtpV1Payload::Callback;
use rfc5321::{Command, XClientParameter};
use std::time::Duration;

#[tokio::test]
async fn xclient_switch_from_addr() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let tracer = daemon.trace_server().await?;

    let props = client.ehlo_lhlo("there", false).await?;
    eprintln!("{props:#?}");
    let xclient_params: Vec<&str> = props
        .get("XCLIENT")
        .as_ref()
        .unwrap()
        .param
        .as_ref()
        .unwrap()
        .split(" ")
        .collect();
    assert!(xclient_params.contains(&"ADDR"), "missing ADDR");
    assert!(xclient_params.contains(&"PORT"), "missing PORT");

    let xclient_result = client
        .send_command(&Command::XClient(vec![
            XClientParameter {
                name: "ADDR".to_string(),
                value: "42.42.42.42".to_string(),
            },
            XClientParameter {
                name: "PORT".to_string(),
                value: "42".to_string(),
            },
        ]))
        .await?;
    eprintln!("{xclient_result:#?}");
    k9::assert_equal!(xclient_result.code, 220);

    let props_after = client.ehlo_lhlo("there", false).await?;
    eprintln!("{props_after:#?}");
    assert!(
        props_after.get("XCLIENT").is_none(),
        "no longer advertising xclient to new ip"
    );

    // The listener config in source.lua only enables allow_xclient
    // for loopback, so we should not be able to change the ip again now
    let xclient_result_after = client
        .send_command(&Command::XClient(vec![XClientParameter {
            name: "ADDR".to_string(),
            value: "100.100.100.100".to_string(),
        }]))
        .await?;
    eprintln!("{xclient_result_after:#?}");
    k9::assert_equal!(xclient_result_after.code, 550);
    k9::assert_equal!(xclient_result_after.content, "insufficient authorization");

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
    k9::assert_equal!(
        final_meta.get("received_from").unwrap().to_string(),
        "\"42.42.42.42:42\""
    );
    assert!(final_meta.get("orig_received_from").is_some());

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}

#[tokio::test]
async fn xclient_switch_via_addr() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let tracer = daemon.trace_server().await?;

    let props = client.ehlo_lhlo("there", false).await?;
    eprintln!("{props:#?}");
    let xclient_params: Vec<&str> = props
        .get("XCLIENT")
        .as_ref()
        .unwrap()
        .param
        .as_ref()
        .unwrap()
        .split(" ")
        .collect();
    assert!(xclient_params.contains(&"DESTADDR"), "missing DESTADDR");
    assert!(xclient_params.contains(&"DESTPORT"), "missing DESTPORT");

    let xclient_result = client
        .send_command(&Command::XClient(vec![
            XClientParameter {
                name: "DESTADDR".to_string(),
                value: "42.42.42.42".to_string(),
            },
            XClientParameter {
                name: "DESTPORT".to_string(),
                value: "42".to_string(),
            },
        ]))
        .await?;
    eprintln!("{xclient_result:#?}");
    k9::assert_equal!(xclient_result.code, 220);
    // Verify that the banner changed; there is a `via` block that
    // will match our new via address that will use a distinct banner
    assert!(xclient_result
        .content
        .ends_with(" what do you get when you multiply six by nine?"));

    let props_after = client.ehlo_lhlo("there", false).await?;
    eprintln!("{props_after:#?}");
    assert!(
        props_after.get("XCLIENT").is_some(),
        "still advertising xclient"
    );

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
    k9::assert_equal!(
        final_meta.get("received_via").unwrap().to_string(),
        "\"42.42.42.42:42\""
    );
    assert!(final_meta.get("orig_received_via").is_some());

    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}
