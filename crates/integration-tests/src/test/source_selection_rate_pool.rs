use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::RecordType;
use std::time::Duration;

/// Check that setting source_selection_rate to 2/day for just one of the
/// sources in the pool results in only 2 messages being sent via that
/// source.  The remainder should go through the other source in that pool
/// We use `warming` as the pool name here; source.lua knows that it should
/// generate two sources named `warming_a` and `warming_b` for that source.
/// We limit `warming_a` to `2/day`.
#[tokio::test]
async fn source_selection_rate_pool() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_SOURCE_SELECTION_RATE_WARMING_A", "2/day"),
        ("KUMOD_POOL_NAME", "warming"),
    ])
    .await
    .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;

    let body = generate_message_text(1024, 78);

    const NUM_MSGS: usize = 10;

    for _ in 0..NUM_MSGS {
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await
        .context("send message")?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);
    }

    daemon
        .wait_for_maildir_count(NUM_MSGS, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let records = daemon.source.collect_logs()?;
    let mut receptions = 0;
    let mut delivery_a = 0;
    let mut delivery_b = 0;

    for record in &records {
        match record.kind {
            RecordType::Reception => {
                receptions += 1;
            }
            RecordType::Delivery => match record.egress_source.as_deref().unwrap() {
                "warming_a" => {
                    delivery_a += 1;
                }
                "warming_b" => {
                    delivery_b += 1;
                }
                wat => panic!("unexpected source {wat} in {record:#?}"),
            },
            _ => {
                panic!("unexpected record: {record:#?}");
            }
        }
    }

    assert_equal!(receptions, 10);
    assert_equal!(delivery_a, 2);
    assert_equal!(delivery_b, 8);

    Ok(())
}
