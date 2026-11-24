use crate::kumod::{DaemonWithMaildir, KumoDaemon, MailGenParams};
use anyhow::Context;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error, Message},
};

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
        for level in ["protocol", "operational", "envelope", "content", "secrets"] {
            let mut url = format!(
                "ws://{}/api/admin/trace-smtp-{direction}/v1",
                endpoint.listener("http")
            );
            if level != "secrets" {
                url.push_str(&format!("?detail={level}"));
            }
            let (mut ws, response) = connect_async(url).await?;
            assert_eq!(response.headers()["kumo-trace-detail"], level);
            ws.send(Message::text("{}")).await?;
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
    for (direction, level, reply_key, mut ws) in traces {
        let transcript = timeout(Duration::from_secs(15), async {
            let mut text = String::new();
            let mut in_data = false;
            loop {
                let frame = ws
                    .next()
                    .await
                    .context("trace closed before DATA reply")??
                    .into_text()?;
                let event: Value = serde_json::from_str(&frame)?;
                text.push_str(&frame);
                for line in event["payload"][reply_key].as_str().unwrap_or("").lines() {
                    if in_data && line.starts_with("250") {
                        return anyhow::Ok(text);
                    }
                    in_data |= line.starts_with("354");
                }
            }
        })
        .await??;
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
async fn smtp_trace_detail_defaults_to_envelope() -> anyhow::Result<()> {
    let mut daemon = KumoDaemon::spawn_maildir().await?;
    for direction in ["server", "client"] {
        let url = format!(
            "ws://{}/api/admin/trace-smtp-{direction}/v1",
            daemon.listener("http")
        );
        let (_, response) = connect_async(&url).await?;
        assert_eq!(response.headers()["kumo-trace-detail"], "envelope");
        assert!(
            matches!(connect_async(format!("{url}?detail=content")).await, Err(Error::Http(r)) if r.status().as_u16() == 403)
        );
    }
    daemon.stop().await?;
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
        let url = format!(
            "ws://{}/api/admin/trace-smtp-{direction}/v1",
            daemon.listener("http")
        );
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
    let client_url = format!(
        "ws://{}/api/admin/trace-smtp-client/v1",
        daemon.listener("http")
    );
    let (mut ws, _) = connect_async(client_url).await?;
    ws.send(Message::text(r#"{"rcpt_to":["user@example.com"]}"#))
        .await?;
    assert!(
        matches!(timeout(Duration::from_secs(5), ws.next()).await?, Some(Ok(Message::Close(Some(f)))) if u16::from(f.code) == 1008)
    );
    daemon.stop().await?;
    Ok(())
}

#[tokio::test]
async fn smtp_trace_detail_protocol_id_survives_xclient() -> anyhow::Result<()> {
    use rfc5321::{Command, XClientParameter};
    let mut daemon = DaemonWithMaildir::start().await?;
    let url = format!(
        "ws://{}/api/admin/trace-smtp-server/v1?detail=protocol",
        daemon.source.listener("http")
    );
    let (mut ws, _) = connect_async(url).await?;
    ws.send(Message::text("{}")).await?;
    let mut smtp = daemon.smtp_client().await?;
    let switched = smtp
        .send_command(&Command::XClient(vec![XClientParameter {
            name: "ADDR".into(),
            value: "192.0.2.42".into(),
        }]))
        .await?;
    assert_eq!(switched.code, 220);
    assert_eq!(smtp.send_command(&Command::Quit).await?.code, 221);
    timeout(Duration::from_secs(5), async {
        let mut session = None;
        loop {
            let frame = ws
                .next()
                .await
                .context("missing Closed event")??
                .into_text()?;
            let event: Value = serde_json::from_str(&frame)?;
            let id = event["conn_meta"]["id"]
                .as_str()
                .context("missing protocol trace id")?;
            if event["payload"] == "Connected" {
                session = Some(id.to_owned());
            }
            assert_eq!(session.as_deref(), Some(id));
            if event["payload"] == "Closed" {
                return anyhow::Ok(());
            }
        }
    })
    .await??;
    daemon.stop_both().await?;
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
    let url = format!(
        "ws://{}/api/admin/trace-smtp-server/v1?detail=content",
        daemon.listener("http")
    );
    let (mut ws, _) = connect_async(url).await?;
    ws.send(Message::text("{}")).await?;
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
