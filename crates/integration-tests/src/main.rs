#[cfg(test)]
mod kumod;
#[cfg(test)]
mod webhook;

fn main() {
    println!("Run me via `cargo nextest run` or `cargo test`");
}

#[cfg(test)]
mod test {
    use super::kumod::*;
    use k9::assert_equal;
    use kumo_log_types::RecordType::{Bounce, TransientFailure};
    use mailparse::MailHeaderMap;
    use rfc5321::*;
    use std::time::Duration;

    #[tokio::test]
    async fn temp_fail() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        let response = MailGenParams {
            recip: Some("tempfail@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon.stop_both().await?;
        let delivery_summary = daemon.dump_logs()?;
        k9::snapshot!(
            delivery_summary,
            "
DeliverySummary {
    source_counts: {
        Reception: 1,
        TransientFailure: 1,
    },
    sink_counts: {},
}
"
        );
        Ok(())
    }

    #[tokio::test]
    async fn perm_fail() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        let response = MailGenParams {
            recip: Some("permfail@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Bounce).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon.stop_both().await?;
        let delivery_summary = daemon.dump_logs()?;
        k9::snapshot!(
            delivery_summary,
            "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Bounce: 1,
    },
    sink_counts: {},
}
"
        );
        Ok(())
    }

    /// test maximum line length
    #[tokio::test]
    async fn max_line_length() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        for n in [128, 512, 1024, 2048, 4192, 1_000_000, 10_000_000] {
            eprintln!("Body length: {n}");
            let body = generate_nonsense_string(n);
            let response = client.send_command(&Command::Noop(Some(body))).await?;
            if n > 998 {
                assert_equal!(response.code, 500);
                assert_equal!(
                    response.enhanced_code,
                    Some(EnhancedStatusCode {
                        class: 5,
                        subject: 2,
                        detail: 3
                    })
                );
                assert_equal!(response.content, "line too long");
            } else {
                assert_equal!(response.code, 250);
            }
        }
        daemon.stop_both().await?;
        Ok(())
    }

    /// Verify that what we send in transits through and is delivered
    /// into the maildir at the other end with the same content
    #[tokio::test]
    async fn end_to_end() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;

        eprintln!("sending message");
        let mut client = daemon.smtp_client().await?;

        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_maildir_count(1, Duration::from_secs(10))
            .await;

        daemon.stop_both().await?;
        println!("Stopped!");

        let delivery_summary = daemon.dump_logs()?;
        k9::snapshot!(
            delivery_summary,
            "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
        );

        let mut messages = daemon.extract_maildir_messages()?;

        assert_equal!(messages.len(), 1);
        let parsed = messages[0].parsed()?;
        println!("headers: {:?}", parsed.headers);

        assert!(parsed.headers.get_first_header("Received").is_some());
        assert!(parsed.headers.get_first_header("X-KumoRef").is_some());
        assert_equal!(
            parsed.headers.get_first_value("From").unwrap(),
            "<sender@example.com>"
        );
        assert_equal!(
            parsed.headers.get_first_value("To").unwrap(),
            "<recip@example.com>"
        );
        assert_equal!(
            parsed.headers.get_first_value("Subject").unwrap(),
            "Hello! This is a test"
        );
        assert_equal!(parsed.get_body()?, body);

        Ok(())
    }

    /// Verify that what we send in transits through and is delivered
    /// into the maildir at the other end with the same content,
    /// and that the webhook logging is also used and captures
    /// the correct amount of records
    #[tokio::test]
    async fn end_to_end_with_webhook() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildirAndWebHook::start().await?;

        eprintln!("sending message");
        let mut client = daemon.with_maildir.smtp_client().await?;

        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .with_maildir
            .wait_for_maildir_count(1, Duration::from_secs(10))
            .await;

        daemon
            .wait_for_webhook_record_count(2, Duration::from_secs(10))
            .await;

        daemon.stop().await?;
        println!("Stopped!");

        let delivery_summary = daemon.with_maildir.dump_logs()?;
        k9::snapshot!(
            delivery_summary,
            "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 3,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
        );

        let webhook_summary = daemon.webhook.dump_logs();
        k9::snapshot!(
            webhook_summary,
            "
Ok(
    {
        Reception: 1,
        Delivery: 1,
    },
)
"
        );

        let mut messages = daemon.with_maildir.extract_maildir_messages()?;

        assert_equal!(messages.len(), 1);
        let parsed = messages[0].parsed()?;
        println!("headers: {:?}", parsed.headers);

        assert!(parsed.headers.get_first_header("Received").is_some());
        assert!(parsed.headers.get_first_header("X-KumoRef").is_some());
        assert_equal!(
            parsed.headers.get_first_value("From").unwrap(),
            "<sender@example.com>"
        );
        assert_equal!(
            parsed.headers.get_first_value("To").unwrap(),
            "<recip@example.com>"
        );
        assert_equal!(
            parsed.headers.get_first_value("Subject").unwrap(),
            "Hello! This is a test"
        );
        assert_equal!(parsed.get_body()?, body);

        Ok(())
    }
}
