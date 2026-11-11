use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use kumo_api_types::InspectQueueV1Request;
use std::time::Duration;

const DOMAIN: &str = "foo.mx-sink.wezfurlong.org";
/// How far into the future the throttle_insert_ready_queue handler in
/// source.lua pushes the message.  This is deliberately much larger than
/// the 60 second jitter window that requeue_message_internal applies when
/// it is not given an explicit delay, so that a regression is unambiguous.
const DELAY_SECS: i64 = 3600;

/// A due time set by a throttle_insert_ready_queue handler must be kept
/// when kumod requeues the message, rather than being replaced by
/// now + rand(0..60s). <https://github.com/KumoCorp/kumomta/issues/593>
#[tokio::test]
async fn throttle_insert_ready_delay_is_honored() -> anyhow::Result<()> {
    // The default SingletonTimerWheel strategy is used here because it is
    // the one whose queued messages are visible via the inspect-sched-q API
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_THROTTLE_INSERT_READY_DELAY", DELAY_SECS.to_string())
        .start()
        .await?;

    let mut client = daemon.smtp_client().await?;
    let response = MailGenParams {
        recip: Some(&format!("deferred@{DOMAIN}")),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    let api = daemon.source.api_client();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let due = loop {
        let resp = api
            .admin_inspect_sched_q_v1(&InspectQueueV1Request {
                queue_name: DOMAIN.to_string(),
                want_body: false,
                limit: None,
            })
            .await;
        if let Ok(resp) = resp {
            if let Some(due) = resp.messages.first().and_then(|m| m.message.due) {
                break due - resp.now;
            }
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the deferred message to appear in the scheduled queue"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    daemon.stop_both().await?;

    let due_secs = due.num_seconds();
    println!("message is due in {due_secs}s");
    anyhow::ensure!(
        due_secs > DELAY_SECS - 60 && due_secs <= DELAY_SECS,
        "expected the message to be due in ~{DELAY_SECS}s, but it is due in {due_secs}s"
    );

    Ok(())
}
