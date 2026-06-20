use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_api_types::{AbortReadyQConnV1Request, DispatcherPhase, InspectReadyQV1Request};
use std::time::Duration;
use uuid::Uuid;

/// Drive a Lua dispatcher whose send blocks for 60s, observe it via
/// the inspect-ready-q API, then abort it via the abort-ready-q-conn
/// API. Verifies the wire shape end-to-end: queue identity, dispatcher
/// summary fields, and that the abort returns success when matching
/// and 404 when not.
#[tokio::test]
async fn inspect_and_abort() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-inspect.lua")
        .start()
        .await
        .context("DaemonWithMaildirOptions::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams {
        recip: Some("victim@inspect.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250, "unexpected response: {response:?}");

    let queue_name = "unspecified->inspect.example.com@lua:make.slow_send".to_string();
    let api = daemon.source.api_client();

    // Wait for the dispatcher to spawn and be observable.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let dispatcher_session_id: Uuid = loop {
        let resp = api
            .admin_inspect_ready_q_v1(&InspectReadyQV1Request {
                queue_name: queue_name.clone(),
            })
            .await;
        if let Ok(resp) = resp {
            if let Some(d) = resp.dispatchers.first() {
                anyhow::ensure!(resp.queue_name == queue_name);
                anyhow::ensure!(resp.egress_source == "unspecified");
                anyhow::ensure!(resp.protocol == "lua");
                anyhow::ensure!(resp.state.connection_count >= 1);
                anyhow::ensure!(matches!(
                    d.phase,
                    DispatcherPhase::DeliveringMessage
                        | DispatcherPhase::AttemptingConnection
                        | DispatcherPhase::Starting
                ));
                break d.session_id;
            }
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "timed out waiting for dispatcher to appear in inspect-ready-q"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    // Abort with a bogus session_id first: should 404.
    let bogus = Uuid::nil();
    let bogus_result = api
        .admin_abort_ready_q_conn_v1(&AbortReadyQConnV1Request {
            queue_name: queue_name.clone(),
            session_id: bogus,
        })
        .await;
    anyhow::ensure!(
        bogus_result.is_err(),
        "bogus session abort should have failed: {bogus_result:?}"
    );

    // Now the real one: should succeed.
    let abort_resp = api
        .admin_abort_ready_q_conn_v1(&AbortReadyQConnV1Request {
            queue_name: queue_name.clone(),
            session_id: dispatcher_session_id,
        })
        .await?;
    anyhow::ensure!(
        abort_resp.contains("aborted"),
        "expected 'aborted' in response, got: {abort_resp}"
    );

    // After the abort the connection slot should drain.
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(10),
            |m| {
                m.name().as_str() == "connection_count"
                    && m.labels()
                        .get("service")
                        .map(|s| s.contains("make.slow_send"))
                        .unwrap_or(false)
            },
            |values| values.iter().sum::<f64>() == 0.0,
        )
        .await
        .context("waiting for connection slot to drain after abort")?;

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}


