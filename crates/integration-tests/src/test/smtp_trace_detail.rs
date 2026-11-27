use crate::kumod::{DaemonWithMaildir, KumoDaemon, MailGenParams};
use anyhow::Context;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type TraceSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn trace_url(daemon: &KumoDaemon, direction: &str) -> String {
    format!(
        "ws://{}/api/admin/trace-smtp-{direction}/v1",
        daemon.listener("http")
    )
}

async fn subscribe(url: &str, expected_level: &str, request: Value) -> anyhow::Result<TraceSocket> {
    let (mut ws, response) = connect_async(url).await?;
    assert_eq!(response.headers()["kumo-trace-detail"], expected_level);
    ws.send(Message::text(request.to_string())).await?;
    Ok(ws)
}

async fn through_data_reply(
    mut ws: TraceSocket,
    reply_key: &str,
    level: &str,
    mut observe: impl FnMut(&Value),
) -> anyhow::Result<()> {
    timeout(Duration::from_secs(30), async {
        let mut in_data = false;
        let mut partial = String::new();
        while let Some(frame) = ws.next().await {
            let mut event: Value = serde_json::from_str(&frame?.into_text()?)?;
            if level == "secrets" {
                // Raw reads in either direction are socket chunks, not SMTP lines.
                if let Some(chunk) = event["payload"]["Read"].as_str() {
                    partial.push_str(chunk);
                    let Some(end) = partial.rfind('\n') else {
                        continue;
                    };
                    let tail = partial.split_off(end + 1);
                    event["payload"]["Read"] = std::mem::replace(&mut partial, tail).into();
                }
            }
            observe(&event);
            for line in event["payload"][reply_key].as_str().unwrap_or("").lines() {
                if in_data && line.starts_with("250") {
                    return Ok(());
                }
                in_data |= line.starts_with("354");
            }
        }
        anyhow::bail!("trace closed before final DATA reply")
    })
    .await?
}

#[tokio::test]
async fn smtp_trace_detail_on_the_wire() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_TEST_MAX_SMTP_TRACE_DETAIL", "secrets"),
        ("KUMOD_SMTP_AUTH_USERNAME", "daniel"),
        ("KUMOD_SMTP_AUTH_PASSWORD", "tiger"),
    ])
    .await?;
    // Observe both ends of the *same* authenticated delivery at each level.
    let mut traces = vec![];
    for (direction, endpoint, reply_key) in [
        ("client", &daemon.source, "Read"),
        ("server", &daemon.sink, "Write"),
    ] {
        let url = trace_url(endpoint, direction);
        for level in ["protocol", "operational", "envelope", "content", "secrets"] {
            let ws = subscribe(&format!("{url}?detail={level}"), level, json!({})).await?;
            traces.push((direction, level, reply_key, ws));
        }
    }
    let mut client = daemon.smtp_client().await?;
    let msg = MailGenParams {
        recip: Some("trace-recipient@example.com"),
        body: Some("trace-body-marker"),
        ..Default::default()
    };
    assert_eq!(msg.send(&mut client).await?.code, 250);
    for (direction, level, reply_key, ws) in traces {
        let mut transcript = String::new();
        through_data_reply(ws, reply_key, level, |event| {
            transcript.push_str(&event.to_string())
        })
        .await?;
        let has = |needle: &str, expected: bool| {
            assert_eq!(
                transcript.contains(needle),
                expected,
                "{direction} {level}: {needle}"
            )
        };
        has("AUTH PLAIN", true);
        has("AGRhbmllbAB0aWdlcg==", level == "secrets"); // the fixture's SASL initial response
        has(
            "RCPT TO:<trace-recipient@example.com>",
            matches!(level, "envelope" | "content" | "secrets"),
        );
        has("trace-recipient@example.com", level != "protocol"); // also in the sink's ordinary 250 reply
        has("trace-body-marker", matches!(level, "content" | "secrets"));
        has("127.0.0.1", level != "protocol");
    }
    daemon.stop_both().await?;
    Ok(())
}

#[tokio::test]
async fn smtp_trace_detail_oversized_data_does_not_end_the_trace() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_TEST_MAX_SMTP_TRACE_DETAIL", "content"),
        ("KUMOD_LOG", "info"),
    ])
    .await?;
    let mut traces = vec![];
    for (direction, endpoint, reply, preview) in [
        ("client", &daemon.source, "Read", "AbbreviatedWrite"),
        ("server", &daemon.sink, "Write", "AbbreviatedRead"),
    ] {
        let url = trace_url(endpoint, direction);
        for terse in [false, true] {
            let ws = subscribe(&url, "content", json!({"terse": terse})).await?;
            traces.push((ws, terse, reply, preview));
        }
    }
    // Quotes double in JSON: the SMTP message fits in 16 MiB, but its full trace event does not.
    let line = "\"".repeat(900) + "\r\n";
    let message = "Subject: large trace\r\n\r\n".to_owned() + &line.repeat(9400);
    let mut client = daemon.smtp_client().await?;
    let response = MailGenParams {
        full_content: Some(&message),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    assert_eq!(response.code, 250);
    for (ws, terse, reply, preview) in traces {
        let mut omitted = false;
        let mut saw_preview = false;
        through_data_reply(ws, reply, "content", |event| {
            let payload = &event["payload"];
            omitted |= payload["Diagnostic"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("SMTP DATA omitted"));
            saw_preview |= payload[preview]["len"]
                .as_u64()
                .is_some_and(|n| n >= message.len() as u64);
        })
        .await?;
        assert_eq!(omitted, !terse);
        assert_eq!(saw_preview, terse);
    }
    daemon.stop_both().await?;
    Ok(())
}

#[tokio::test]
async fn smtp_trace_detail_kcli_reports_read_errors() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    for command in ["trace-smtp-client", "trace-smtp-server"] {
        let cli = tokio::process::Command::new(crate::kumod::target_bin("kcli")?)
            .args(["--endpoint", &endpoint, command])
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        timeout(Duration::from_secs(5), async {
            let (tcp, _) = listener.accept().await?;
            let mut ws = tokio_tungstenite::accept_async(tcp).await?;
            assert!(ws.next().await.context("missing subscription")??.is_text());
            drop(ws); // TCP EOF without a WebSocket closing handshake is a read error.
            let output = cli.wait_with_output().await?;
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("closing handshake"));
            anyhow::Ok(())
        })
        .await??;
    }
    Ok(())
}

#[tokio::test]
async fn smtp_trace_detail_listener_caps_the_default_and_explicit_request() -> anyhow::Result<()> {
    let mut daemon = KumoDaemon::spawn_maildir_env(vec![(
        "KUMOD_TEST_MAX_SMTP_TRACE_DETAIL".into(),
        "operational".into(),
    )])
    .await?;
    for direction in ["server", "client"] {
        let url = trace_url(&daemon, direction);
        for (query, expected) in [("", "operational"), ("?detail=protocol", "protocol")] {
            let (_, response) = connect_async(format!("{url}{query}")).await?;
            assert_eq!(response.headers()["kumo-trace-detail"], expected);
        }
        assert!(
            matches!(connect_async(format!("{url}?detail=content")).await, Err(Error::Http(r)) if r.status().as_u16() == 403)
        );
    }
    let output = tokio::process::Command::new(crate::kumod::target_bin("kcli")?)
        .args([
            "--endpoint",
            &format!("http://{}", daemon.listener("http")),
            "trace-smtp-server",
            "--trace-detail",
            "content",
        ])
        .output()
        .await?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("403"));
    let refused = tokio::process::Command::new(crate::kumod::target_bin("kcli")?)
        .args([
            "--endpoint",
            &format!("http://{}", daemon.listener("http")),
            "trace-smtp-client",
            "--rcpt-to",
            "user@example.com",
        ])
        .kill_on_drop(true)
        .output();
    let output = timeout(Duration::from_secs(5), refused).await??;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "SMTP trace closed: trace filter requires envelope detail on this listener (1008)"
    ));
    daemon.stop().await?;
    Ok(())
}

#[tokio::test]
async fn smtp_trace_detail_filters_on_mx_before_projecting_protocol_output() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![(
        "KUMOD_TEST_MAX_SMTP_TRACE_DETAIL",
        "operational",
    )])
    .await?;
    let url = format!("{}?detail=protocol", trace_url(&daemon.source, "client"));
    let control = subscribe(&url, "protocol", json!({})).await?;
    let filtered = subscribe(
        &url,
        "protocol",
        json!({"mx_host": ["localhost-1", "localhost-2"]}),
    )
    .await?;
    let mut smtp = daemon.smtp_client().await?;

    // Drain the first delivery on the unfiltered subscription before starting the matching one.
    let first = MailGenParams {
        recip: Some("first@example.com"),
        ..Default::default()
    };
    assert_eq!(first.send(&mut smtp).await?.code, 250);
    let mut first_id = None;
    through_data_reply(control, "Read", "protocol", |event| {
        if event["payload"] == "Connected" {
            first_id = Some(event["conn_meta"]["id"].as_str().unwrap().to_owned());
        }
    })
    .await?;
    let first_id = first_id.context("missing first connection")?;
    let second = MailGenParams {
        recip: Some("second@two-hosts.example.com"),
        ..Default::default()
    };
    assert_eq!(second.send(&mut smtp).await?.code, 250);
    let mut connected = false;
    through_data_reply(filtered, "Read", "protocol", |event| {
        let meta = &event["conn_meta"];
        assert_ne!(meta["id"].as_str().unwrap(), first_id);
        for field in ["mx_host", "mx_address", "mx_plan"] {
            assert!(meta.get(field).is_none(), "{field} in protocol output");
        }
        connected |= event["payload"] == "Connected";
    })
    .await?;
    assert!(connected);
    daemon.stop_both().await?;
    Ok(())
}

#[tokio::test]
async fn smtp_trace_detail_metadata_is_scoped_to_the_http_listener() -> anyhow::Result<()> {
    let mut daemon = KumoDaemon::spawn_with_policy("smtp-trace-meta.lua").await?;
    let [default, configured] = daemon.listeners["http"][..] else {
        anyhow::bail!("expected two HTTP listeners");
    };
    let mut traces = vec![];
    for (addr, level, route, customer) in [
        (default, "envelope", None, None),
        (configured, "operational", Some("bulk"), None),
        (configured, "envelope", Some("bulk"), Some("example")),
    ] {
        let url = format!("ws://{addr}/api/admin/trace-smtp-server/v1?detail={level}");
        traces.push((subscribe(&url, level, json!({})).await?, route, customer));
    }
    daemon.smtp_client("localhost").await?;
    for (mut ws, route, customer) in traces {
        let frame = timeout(Duration::from_secs(5), ws.next())
            .await?
            .context("trace closed before Connected")??;
        let event: Value = serde_json::from_str(&frame.into_text()?)?;
        assert_eq!(event["payload"], "Connected");
        let meta = &event["conn_meta"];
        assert_eq!(meta.get("x_route"), route.map(Value::from).as_ref());
        assert_eq!(meta.get("x_customer"), customer.map(Value::from).as_ref());
        assert!(meta.get("x_auth_context").is_none());
        assert!(meta.get("legacy_key").is_none());
    }
    daemon.stop().await?;
    Ok(())
}

#[tokio::test]
async fn smtp_trace_detail_join_during_auth_continuation() -> anyhow::Result<()> {
    use rfc5321::{Command, TlsOptions};
    let mut daemon = KumoDaemon::spawn_maildir_env(vec![(
        "KUMOD_TEST_MAX_SMTP_TRACE_DETAIL".into(),
        "content".into(),
    )])
    .await?;
    let mut smtp = daemon.smtp_client("localhost").await?;
    smtp.starttls(TlsOptions {
        insecure: true,
        ..Default::default()
    })
    .await?;
    smtp.ehlo("localhost").await?;
    let challenge = smtp
        .send_command(&Command::Auth {
            sasl_mech: "PLAIN".into(),
            initial_response: None,
        })
        .await?;
    assert_eq!(challenge.code, 334);
    // This subscriber never saw AUTH. The parser, not the consumer, must identify the next line.
    let url = format!("{}?detail=content", trace_url(&daemon, "server"));
    let mut ws = subscribe(&url, "content", json!({})).await?;
    assert_eq!(
        smtp.send_command(&Command::Unknown("AGRhbmllbAB0aWdlcg==".into()))
            .await?
            .code,
        235
    );
    timeout(Duration::from_secs(5), async {
        let mut saw_continuation = false;
        loop {
            let frame = ws
                .next()
                .await
                .context("trace closed before AUTH reply")??
                .into_text()?;
            assert!(!frame.contains("AGRhbmllbAB0aWdlcg=="));
            let event: Value = serde_json::from_str(&frame)?;
            saw_continuation |= event["payload"]["Read"] == "[AUTH continuation]";
            if event["payload"]["Write"]
                .as_str()
                .is_some_and(|s| s.starts_with("235"))
            {
                assert!(saw_continuation);
                return anyhow::Ok(());
            }
        }
    })
    .await??;
    daemon.stop().await?;
    Ok(())
}
