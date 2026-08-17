use crate::kumod::{generate_message_text, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use std::time::Duration;

/// Regression test for <https://github.com/KumoCorp/kumomta/issues/533>.
///
/// Verify that mailing to an IDNA domain works regardless of whether the
/// client specifies the recipient domain using the unicode (U-label) form
/// or the ASCII-Compatible Encoding (ACE / punycode) form. The source.lua
/// is configured so that delivery for this domain goes to an MX whose
/// hostname is the U-label form, exercising the punycode normalization
/// path through the TLS server name parser during STARTTLS.
#[tokio::test]
async fn idna_starttls() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-idna.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let body = generate_message_text(1024, 78);

    for recip in [
        "unicode@münchen.mx-sink.wezfurlong.org",
        "ace@xn--mnchen-3ya.mx-sink.wezfurlong.org",
    ] {
        eprintln!("sending message to {recip}");
        let mut client = daemon.smtp_client().await.context("make smtp_client")?;
        let response = MailGenParams {
            body: Some(&body),
            recip: Some(recip),
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

    let delivery_summary = daemon.dump_logs().await.context("dump_logs")?;
    k9::snapshot!(
        delivery_summary,
        "
DeliverySummary {
    source_counts: {
        Reception: 2,
        Delivery: 2,
    },
    sink_counts: {
        Reception: 2,
        Delivery: 2,
    },
}
"
    );

    let messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 2);

    Ok(())
}
