use crate::kumod::{generate_message_text, DaemonWithMaildir, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::RecordType;
use std::time::Duration;

/// Check that setting source_selection_rate to 2/day limits us to delivering
/// only 2 out of the 10 messages we plan to send here, and that we get the
/// expected TransientFailure log for the messages that can't be delivered.
/// We have just a single source in this example.
#[tokio::test]
async fn source_selection_rate() -> anyhow::Result<()> {
    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_SOURCE_SELECTION_RATE", "2/day")])
            .await
            .context("DaemonWithMaildir::start")?;

    eprintln!("sending message");
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
        .wait_for_maildir_count(2, Duration::from_secs(10))
        .await;

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let records = daemon.source.collect_logs().await?;
    let mut receptions = 0;
    let mut delivery = 0;
    let mut trans_fail = 0;

    for record in &records {
        match record.kind {
            RecordType::Reception => {
                receptions += 1;
            }
            RecordType::Delivery => {
                delivery += 1;
            }
            RecordType::TransientFailure => {
                trans_fail += 1;
                eprintln!("Considering TransientFailure: {record:#?}");
                assert_equal!(
                    record.response.content,
                    "KumoMTA internal: no sources for example.com pool=`unspecified` \
                    are eligible for selection at this time"
                );
            }
            _ => {
                panic!("unexpected record: {record:#?}");
            }
        }
    }

    assert_equal!(receptions, 10);
    assert_equal!(delivery, 2);
    assert_equal!(trans_fail, 8);

    Ok(())
}
