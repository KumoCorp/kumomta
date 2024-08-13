#[cfg(test)]
mod kumod;
mod rabbit;
#[cfg(test)]
mod tsa;
#[cfg(test)]
mod webhook;

fn main() {
    println!("Run me via `cargo nextest run` or `cargo test`");
}

#[cfg(test)]
mod test {
    use super::kumod::*;
    use anyhow::Context;
    use k9::assert_equal;
    use kumo_api_types::{SuspendReadyQueueV1ListEntry, SuspendV1ListEntry, SuspendV1Response};
    use kumo_log_types::RecordType;
    use kumo_log_types::RecordType::{Bounce, Delivery, Reception, TransientFailure};
    use mailparsing::DecodedBody;
    use rfc5321::*;
    use serde_json::json;
    use std::collections::BTreeMap;
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
    sink_counts: {
        Rejection: 2,
    },
}
"
        );

        k9::snapshot!(
            daemon.source.accounting_stats()?,
            "
AccountingStats {
    received: 1,
    delivered: 0,
}
"
        );
        Ok(())
    }

    #[tokio::test]
    async fn suspend_delivery_ready_q() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        let status: SuspendV1Response = daemon
            .kcli_json([
                "suspend-ready-q",
                "--name",
                "unspecified->mx_list:localhost@smtp_client",
                "--reason",
                "testing",
            ])
            .await?;
        println!("kcli status: {status:?}");

        let response = MailGenParams {
            recip: Some("allow@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        // Allow a little bit of time for a delivery to go through
        // if for some reason suspension is broken
        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Reception).copied().unwrap_or(0) > 0,
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
        k9::snapshot!(
            daemon.source.accounting_stats()?,
            "
AccountingStats {
    received: 1,
    delivered: 0,
}
"
        );
        Ok(())
    }

    #[tokio::test]
    async fn suspend_delivery_ready_q_and_deliver() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        let status: SuspendV1Response = daemon
            .kcli_json([
                "suspend-ready-q",
                "--name",
                "unspecified->mx_list:localhost@smtp_client",
                "--reason",
                "testing",
            ])
            .await?;
        println!("kcli status: {status:?}");

        let response = MailGenParams {
            recip: Some("allow@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        // Allow a little bit of time for a delivery to go through
        // if for some reason suspension is broken
        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Reception).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .kcli(["suspend-ready-q-cancel", "--id", &format!("{}", status.id)])
            .await?;

        // The suspension can add up to 1 minute of jittered delay
        // to the original message. To verify that the suspension
        // has been lifted, we inject a second message.
        let response = MailGenParams {
            recip: Some("allow2@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) == 1,
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
        Reception: 2,
        Delivery: 1,
        TransientFailure: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
        );
        k9::snapshot!(
            daemon.source.accounting_stats()?,
            "
AccountingStats {
    received: 2,
    delivered: 1,
}
"
        );
        Ok(())
    }

    #[tokio::test]
    async fn suspend_delivery_scheduled_q() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        let status: SuspendV1Response = daemon
            .kcli_json(["suspend", "--domain", "example.com", "--reason", "testing"])
            .await?;
        println!("kcli status: {status:?}");

        let response = MailGenParams {
            recip: Some("allow@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        // Allow a little bit of time for a delivery to go through
        // if for some reason suspension is broken
        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Reception).copied().unwrap_or(0) > 0,
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
    },
    sink_counts: {},
}
"
        );
        k9::snapshot!(
            daemon.source.accounting_stats()?,
            "
AccountingStats {
    received: 1,
    delivered: 0,
}
"
        );
        Ok(())
    }

    #[tokio::test]
    async fn suspend_delivery_scheduled_q_and_deliver() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        let status: SuspendV1Response = daemon
            .kcli_json(["suspend", "--domain", "example.com", "--reason", "testing"])
            .await?;
        println!("kcli status: {status:?}");

        let response = MailGenParams {
            recip: Some("allow@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        // Allow a little bit of time for a delivery to go through
        // if for some reason suspension is broken
        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Reception).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .kcli(["suspend-cancel", "--id", &format!("{}", status.id)])
            .await?;

        // The suspension can add up to 1 minute of jittered delay
        // to the original message. To verify that the suspension
        // has been lifted, we inject a second message.
        // That second message should get delivered, while the first
        // remains in the queue
        let response = MailGenParams {
            recip: Some("allow2@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) == 1,
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
        Reception: 2,
        Delivery: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
        );
        k9::snapshot!(
            daemon.source.accounting_stats()?,
            "
AccountingStats {
    received: 2,
    delivered: 1,
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
    sink_counts: {
        Rejection: 2,
    },
}
"
        );
        k9::snapshot!(
            daemon.source.accounting_stats()?,
            "
AccountingStats {
    received: 1,
    delivered: 0,
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
        let mut daemon = DaemonWithMaildir::start()
            .await
            .context("DaemonWithMaildir::start")?;

        eprintln!("sending message");
        let mut client = daemon.smtp_client().await.context("make smtp_client")?;

        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await
        .context("send message")?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_maildir_count(1, Duration::from_secs(10))
            .await;

        daemon.stop_both().await.context("stop_both")?;
        println!("Stopped!");

        daemon.source.check_for_x_and_y_headers_in_logs()?;

        let delivery_summary = daemon.dump_logs().context("dump_logs")?;
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
        k9::snapshot!(
            daemon.source.accounting_stats()?,
            "
AccountingStats {
    received: 1,
    delivered: 1,
}
"
        );

        let mut messages = daemon.extract_maildir_messages()?;

        assert_equal!(messages.len(), 1);
        let parsed = messages[0].parsed()?;
        println!("headers: {:?}", parsed.headers());

        assert!(parsed.headers().get_first("Received").is_some());
        assert!(parsed.headers().get_first("X-KumoRef").is_some());

        // These two headers are added to all MailGenParams generated mail
        assert!(parsed.headers().get_first("X-Test1").is_some());
        assert!(parsed.headers().get_first("X-Another").is_some());

        k9::snapshot!(
            parsed.headers().from().unwrap(),
            r#"
Some(
    MailboxList(
        [
            Mailbox {
                name: None,
                address: AddrSpec {
                    local_part: "sender",
                    domain: "example.com",
                },
            },
        ],
    ),
)
"#
        );
        k9::snapshot!(
            parsed.headers().to().unwrap(),
            r#"
Some(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: None,
                    address: AddrSpec {
                        local_part: "recip",
                        domain: "example.com",
                    },
                },
            ),
        ],
    ),
)
"#
        );
        assert_equal!(
            parsed.headers().subject().unwrap().unwrap(),
            "Hello! This is a test"
        );
        assert_equal!(parsed.body().unwrap(), DecodedBody::Text(body.into()));

        Ok(())
    }

    fn json_string(v: serde_json::Value) -> String {
        serde_json::to_string(&v).unwrap()
    }

    #[tokio::test]
    async fn log_oob_arf() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start_with_env(vec![
            (
                "KUMOD_LISTENER_DOMAIN_MAP",
                &json_string(json!({
                    "example.com": {
                        "relay_to": false,
                        "log_oob": true,
                        "log_arf": true,
                    }

                })),
            ),
            ("KUMOD_RELAY_HOSTS", &json_string(json!([]))),
        ])
        .await
        .context("DaemonWithMaildir::start")?;

        eprintln!("sending message");
        let mut client = daemon.smtp_client().await.context("make smtp_client")?;

        let oob = include_str!("../../kumo-log-types/data/rfc3464/1.eml");
        let arf = include_str!("../../kumo-log-types/data/rfc5965/1.eml");
        for data in [oob, arf] {
            let response = MailGenParams {
                full_content: Some(data),
                ..Default::default()
            }
            .send(&mut client)
            .await
            .context("send message")?;
            eprintln!("{response:?}");
            anyhow::ensure!(response.code == 250);
        }

        {
            // and verify that relaying for !report is not allowed
            let body = generate_message_text(1024, 78);
            let response = MailGenParams {
                body: Some(&body),
                ..Default::default()
            }
            .send(&mut client)
            .await
            .unwrap_err();
            k9::snapshot!(
                response,
                r#"
Rejected(
    Response {
        code: 550,
        enhanced_code: Some(
            EnhancedStatusCode {
                class: 5,
                subject: 7,
                detail: 1,
            },
        ),
        content: "relaying not permitted",
        command: Some(
            ".\r
",
        ),
    },
)
"#
            );
        }

        daemon
            .wait_for_maildir_count(2, Duration::from_secs(10))
            .await;

        daemon.stop_both().await.context("stop_both")?;
        println!("Stopped!");

        let delivery_summary = daemon.dump_logs().context("dump_logs")?;
        k9::snapshot!(
            delivery_summary,
            "
DeliverySummary {
    source_counts: {
        Reception: 2,
        OOB: 1,
        Feedback: 1,
        Rejection: 1,
    },
    sink_counts: {},
}
"
        );
        k9::snapshot!(
            daemon.source.accounting_stats()?,
            "
AccountingStats {
    received: 2,
    delivered: 0,
}
"
        );

        Ok(())
    }

    /// Verify that what we send in transits through and is delivered
    /// into the maildir at the other end with the same content
    #[tokio::test]
    async fn end_to_end_stuffed() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start()
            .await
            .context("DaemonWithMaildir::start")?;

        eprintln!("sending message");
        let mut client = daemon.smtp_client().await.context("make smtp_client")?;

        let body = ".Stuffing required\r\nFor me\r\n";
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await
        .context("send message")?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_maildir_count(1, Duration::from_secs(10))
            .await;

        daemon.stop_both().await.context("stop_both")?;
        println!("Stopped!");

        let delivery_summary = daemon.dump_logs().context("dump_logs")?;
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
        println!("headers: {:?}", parsed.headers());

        assert!(parsed.headers().get_first("Received").is_some());
        assert!(parsed.headers().get_first("X-KumoRef").is_some());
        k9::snapshot!(
            parsed.headers().from().unwrap(),
            r#"
Some(
    MailboxList(
        [
            Mailbox {
                name: None,
                address: AddrSpec {
                    local_part: "sender",
                    domain: "example.com",
                },
            },
        ],
    ),
)
"#
        );
        k9::snapshot!(
            parsed.headers().to().unwrap(),
            r#"
Some(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: None,
                    address: AddrSpec {
                        local_part: "recip",
                        domain: "example.com",
                    },
                },
            ),
        ],
    ),
)
"#
        );
        assert_equal!(
            parsed.headers().subject().unwrap().unwrap(),
            "Hello! This is a test"
        );
        assert_equal!(parsed.body().unwrap(), DecodedBody::Text(body.into()));

        Ok(())
    }

    #[tokio::test]
    async fn auth_deliver() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start_with_env(vec![
            ("KUMOD_SMTP_AUTH_USERNAME", "scott"),
            ("KUMOD_SMTP_AUTH_PASSWORD", "tiger"),
        ])
        .await?;

        let mut client = daemon.smtp_client().await?;

        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
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
        Ok(())
    }

    #[tokio::test]
    async fn auth_deliver_invalid_password() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start_with_env(vec![
            ("KUMOD_SMTP_AUTH_USERNAME", "scott"),
            ("KUMOD_SMTP_AUTH_PASSWORD", "incorrect-password"),
        ])
        .await?;

        let mut client = daemon.smtp_client().await?;

        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
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
        TransientFailure: 1,
    },
    sink_counts: {
        Rejection: 1,
    },
}
"
        );
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

        let mut logged_headers = vec![];
        let mut headers_delivered = vec![];
        for record in daemon.webhook.return_logs() {
            let ordered_headers: BTreeMap<_, _> = record.headers.into_iter().collect();
            if record.kind == RecordType::Reception {
                logged_headers.push(ordered_headers);
            } else if record.kind == RecordType::Delivery {
                headers_delivered.push(ordered_headers);
            }
        }
        k9::snapshot!(
            logged_headers,
            r#"
[
    {
        "Subject": String("Hello! This is a test"),
        "X-Another": String("Another"),
        "X-KumoRef": String("eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20ifQ=="),
        "X-Test1": String("Test1"),
    },
]
"#
        );
        k9::snapshot!(
            headers_delivered,
            r#"
[
    {
        "Subject": String("Hello! This is a test"),
        "X-Another": String("Another"),
        "X-KumoRef": String("eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20ifQ=="),
        "X-Test1": String("Test1"),
    },
]
"#
        );

        let mut messages = daemon.with_maildir.extract_maildir_messages()?;

        assert_equal!(messages.len(), 1);
        let parsed = messages[0].parsed()?;
        println!("headers: {:?}", parsed.headers());

        assert!(parsed.headers().get_first("Received").is_some());
        assert!(parsed.headers().get_first("X-KumoRef").is_some());
        k9::snapshot!(
            parsed.headers().from().unwrap(),
            r#"
Some(
    MailboxList(
        [
            Mailbox {
                name: None,
                address: AddrSpec {
                    local_part: "sender",
                    domain: "example.com",
                },
            },
        ],
    ),
)
"#
        );
        k9::snapshot!(
            parsed.headers().to().unwrap(),
            r#"
Some(
    AddressList(
        [
            Mailbox(
                Mailbox {
                    name: None,
                    address: AddrSpec {
                        local_part: "recip",
                        domain: "example.com",
                    },
                },
            ),
        ],
    ),
)
"#
        );
        assert_equal!(
            parsed.headers().subject().unwrap().unwrap(),
            "Hello! This is a test"
        );
        assert_equal!(parsed.body().unwrap(), DecodedBody::Text(body.into()));

        Ok(())
    }

    #[tokio::test]
    async fn tls_opportunistic_fail() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start_with_env(vec![
            // The default is OpportunisticInsecure; tighten it up a bit
            // so that we fail to deliver to the sink, and verify
            // the result of that attempt below
            ("KUMOD_ENABLE_TLS", "Opportunistic"),
        ])
        .await?;

        let mut client = daemon.smtp_client().await?;

        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
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
        TransientFailure: 1,
    },
    sink_counts: {},
}
"
        );
        Ok(())
    }

    #[tokio::test]
    async fn retry_schedule_timerwheel() -> anyhow::Result<()> {
        retry_schedule_impl("TimerWheel").await
    }
    #[tokio::test]
    async fn retry_schedule_skiplist() -> anyhow::Result<()> {
        retry_schedule_impl("SkipList").await
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

    #[tokio::test]
    async fn tsa_basic_automation() -> anyhow::Result<()> {
        let mut daemon = DaemonWithTsa::start().await?;

        let mut client = daemon.smtp_client().await?;

        let body = generate_message_text(1024, 78);
        let response = MailGenParams {
            body: Some(&body),
            recip: Some("450-go-away@foo.mx-sink.wezfurlong.org"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250);

        daemon
            .with_maildir
            .wait_for_source_summary(
                |summary| {
                    summary.get(&TransientFailure).copied().unwrap_or(0) > 0
                        && summary.get(&Delivery).copied().unwrap_or(0) > 0
                },
                Duration::from_secs(5),
            )
            .await;

        let delivery_summary = daemon.with_maildir.dump_logs()?;
        k9::snapshot!(
            delivery_summary,
            "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
        TransientFailure: 1,
    },
    sink_counts: {
        Rejection: 2,
    },
}
"
        );

        async fn get_suspensions(
            daemon: &DaemonWithTsa,
        ) -> anyhow::Result<Vec<SuspendReadyQueueV1ListEntry>> {
            daemon
                .with_maildir
                .kcli_json(["suspend-ready-q-list"])
                .await
        }

        for _ in 0..5 {
            let status = get_suspensions(&daemon).await?;
            if status.is_empty() {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            } else {
                break;
            }
        }

        // Confirm kumod sees a suspension
        let status = get_suspensions(&daemon).await?;
        println!("kcli status: {status:?}");

        assert!(!status.is_empty(), "did suspend");
        assert_eq!(status[0].name, "unspecified->localhost@smtp_client");
        let reason = &status[0].reason;
        assert!(reason.contains("you said"), "{reason}");

        daemon.stop().await?;
        Ok(())
    }

    #[tokio::test]
    async fn tsa_tenant_suspension() -> anyhow::Result<()> {
        let mut daemon = DaemonWithTsa::start().await?;

        let mut client = daemon.smtp_client().await?;

        let body = "X-Tenant: mytenant\r\nX-Campaign: mycamp\r\n\r\nFoo";
        let response = MailGenParams {
            full_content: Some(body),
            recip: Some("450-suspend-campaign@foo.mx-sink.wezfurlong.org"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        anyhow::ensure!(response.code == 250);

        daemon
            .with_maildir
            .wait_for_source_summary(
                |summary| {
                    summary.get(&TransientFailure).copied().unwrap_or(0) > 0
                        && summary.get(&Delivery).copied().unwrap_or(0) > 0
                },
                Duration::from_secs(5),
            )
            .await;

        let delivery_summary = daemon.with_maildir.dump_logs()?;
        k9::snapshot!(
            delivery_summary,
            "
DeliverySummary {
    source_counts: {
        Reception: 1,
        Delivery: 1,
        TransientFailure: 1,
    },
    sink_counts: {
        Rejection: 2,
    },
}
"
        );

        async fn get_suspensions(
            daemon: &DaemonWithTsa,
        ) -> anyhow::Result<Vec<SuspendV1ListEntry>> {
            daemon.with_maildir.kcli_json(["suspend-list"]).await
        }

        for _ in 0..5 {
            let status = get_suspensions(&daemon).await?;
            if status.is_empty() {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            } else {
                break;
            }
        }

        // Confirm kumod sees a suspension
        let status = get_suspensions(&daemon).await?;
        println!("kcli status: {status:?}");

        assert!(!status.is_empty(), "did suspend");
        let item = &status[0];
        let reason = &item.reason;
        assert!(reason.contains("you said"), "{reason}");
        assert_eq!(item.campaign.as_deref(), Some("mycamp"));
        assert_eq!(item.tenant.as_deref(), Some("mytenant"));

        daemon.stop().await?;
        Ok(())
    }

    #[tokio::test]
    async fn rebind() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        let status: SuspendV1Response = daemon
            .kcli_json(["suspend", "--domain", "example.com", "--reason", "testing"])
            .await?;
        println!("kcli status: {status:?}");

        let response = MailGenParams {
            recip: Some("allow@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Reception).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .kcli([
                "rebind",
                "--domain",
                "example.com",
                "--reason",
                "testing",
                "--data",
                "{\"queue\":\"rebound.com\"}",
            ])
            .await?;

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
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
        Delivery: 1,
        AdminRebind: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
        );
        Ok(())
    }

    #[tokio::test]
    async fn rebind_event_missing() -> anyhow::Result<()> {
        let mut daemon = DaemonWithMaildir::start().await?;
        let mut client = daemon.smtp_client().await?;

        let status: SuspendV1Response = daemon
            .kcli_json(["suspend", "--domain", "example.com", "--reason", "testing"])
            .await?;
        println!("kcli status: {status:?}");

        let response = MailGenParams {
            recip: Some("allow@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Reception).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .kcli([
                "rebind",
                "--domain",
                "example.com",
                "--reason",
                "testing",
                "--trigger-rebind-event",
                "--data",
                "{\"queue\":\"rebound.com\"}",
            ])
            .await?;

        // Grace period for a delivery to go through in case of weirdness.
        // We DON'T expect there to be a delivery
        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
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
    },
    sink_counts: {},
}
"
        );
        Ok(())
    }

    #[tokio::test]
    async fn rebind_event_defined() -> anyhow::Result<()> {
        let mut daemon =
            DaemonWithMaildir::start_with_env(vec![("KUMOD_WANT_REBIND", "1")]).await?;
        let mut client = daemon.smtp_client().await?;

        let status: SuspendV1Response = daemon
            .kcli_json(["suspend", "--domain", "example.com", "--reason", "testing"])
            .await?;
        println!("kcli status: {status:?}");

        let response = MailGenParams {
            recip: Some("allow@example.com"),
            ..Default::default()
        }
        .send(&mut client)
        .await?;
        eprintln!("{response:?}");
        anyhow::ensure!(response.code == 250);

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Reception).copied().unwrap_or(0) > 0,
                Duration::from_secs(5),
            )
            .await;

        daemon
            .kcli([
                "rebind",
                "--domain",
                "example.com",
                "--reason",
                "testing",
                "--trigger-rebind-event",
                "--data",
                "{\"queue\":\"rebound.com\"}",
            ])
            .await?;

        daemon
            .wait_for_source_summary(
                |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
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
        Delivery: 1,
        AdminRebind: 1,
    },
    sink_counts: {
        Reception: 1,
        Delivery: 1,
    },
}
"
        );
        Ok(())
    }
}
