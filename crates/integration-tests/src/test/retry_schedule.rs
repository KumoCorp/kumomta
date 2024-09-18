use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

#[tokio::test]
async fn retry_schedule_timerwheel() -> anyhow::Result<()> {
    retry_schedule_impl("TimerWheel").await
}
#[tokio::test]
async fn retry_schedule_skiplist() -> anyhow::Result<()> {
    retry_schedule_impl("SkipList").await
}
#[tokio::test]
async fn retry_schedule_singleton_wheel() -> anyhow::Result<()> {
    retry_schedule_impl("SingletonTimerWheel").await
}

async fn retry_schedule_impl(strategy: &str) -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_RETRY_INTERVAL", "5s"),
        ("KUMOD_QUEUE_STRATEGY", strategy),
    ])
    .await?;

    let mut client = daemon.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("tempfail@foo.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 1,
            Duration::from_secs(15),
        )
        .await;

    daemon.stop_both().await?;
    println!("Stopped!");

    let records = daemon.source.collect_logs()?;
    let event_times: Vec<_> = records
        .iter()
        .filter_map(|record| match record.kind {
            TransientFailure => Some((record.timestamp - record.created).num_seconds()),
            _ => None,
        })
        .collect();

    println!("***** event_times: {event_times:?}");
    assert!(event_times.len() > 1);

    let mut last = None;
    let mut intervals: Vec<_> = event_times
        .iter()
        .map(|t| {
            let result = match last {
                Some(l) => *t - l,
                None => *t,
            };
            last.replace(*t);
            result
        })
        .collect();

    let first = intervals.remove(0);
    assert!(
        first >= 0 && first <= 1,
        "first is {first} but should be ~0"
    );
    let mut expect = 5;
    for actual in intervals {
        assert!(
            actual >= expect && actual <= (expect + expect / 2),
            "expected {expect} got {actual}"
        );
        expect *= 2;
    }

    Ok(())
}
