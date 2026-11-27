use crate::kumod::{DaemonWithMaildir, KumoArgs, KumoDaemon, MailGenParams};
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use kumo_log_types::ResolvedAddress;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::time::timeout;

#[tokio::test]
async fn disconnect_in_data() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams {
        recip: Some("not-me@two-hosts.example.com"),
        // Cause the sink to 451 disconnect us in DATA.
        // This is to verify that we handle this sort of error during
        // a pipeline send correctly.
        sender: Some("disconnect-in-data-no-421@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    let response = MailGenParams {
        recip: Some("pick-me@two-hosts.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(10),
        )
        .await;

    daemon.stop_both().await?;

    let delivery_summary = daemon.dump_logs().await?;
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
        Rejection: 1,
    },
}
"
    );

    Ok(())
}

#[tokio::test]
async fn empty_data_timeout_requeues_without_trying_next_mx() -> anyhow::Result<()> {
    // Both MX candidates for two-hosts.example.com point at this listener.
    let mx = TcpListener::bind("127.0.0.1:0").await?;
    let mut source = KumoDaemon::spawn(KumoArgs {
        policy_file: "source.lua".into(),
        env: vec![
            (
                "KUMOD_SMTP_SINK_PORT".into(),
                mx.local_addr()?.port().to_string(),
            ),
            ("KUMOD_TRY_NEXT_HOST_ON_TRANSPORT_ERROR".into(), "1".into()),
            ("KUMOD_DATA_DOT_TIMEOUT".into(), "200ms".into()),
        ],
    })
    .await?;
    let mut client = source.smtp_client("localhost").await?;
    let reply = MailGenParams {
        recip: Some("busy@two-hosts.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    assert_eq!(reply.code, 250);
    drop(client);

    let budget = Duration::from_secs(10);
    timeout(budget, async {
        let (socket, _) = mx.accept().await?;
        let mut peer = BufReader::new(socket);
        let mut request = String::new();
        peer.get_mut().write_all(b"220 mx.example\r\n").await?;
        peer.read_line(&mut request).await?;
        assert!(request.starts_with("EHLO "));
        peer.get_mut()
            .write_all(b"250-mx.example\r\n250 PIPELINING\r\n")
            .await?;
        while !request.ends_with("DATA\r\n") {
            anyhow::ensure!(peer.read_line(&mut request).await? > 0, "EOF before DATA");
        }
        peer.get_mut()
            .write_all(b"250 ok\r\n450 busy\r\n354 send data\r\n")
            .await?;
        // Leave the connection open. Read through EOF so we know the client timed out.
        let mut data = String::new();
        peer.read_to_string(&mut data).await?;
        assert_eq!(data, ".\r\n");
        anyhow::Ok(())
    })
    .await??;

    let requeued = source.wait_for_metric(
        budget,
        |m| m.name().as_str() == "scheduled_count" && m.label_is("queue", "two-hosts.example.com"),
        |values| values == [1.0],
    );
    tokio::select! {
        biased;
        _ = mx.accept() => anyhow::bail!("retried on the next MX after an explicit RCPT deferral"),
        result = requeued => result?,
    }
    source.stop().await?;
    let logs = source.collect_logs().await?;
    let tf = logs
        .iter()
        .find(|r| r.kind == TransientFailure)
        .expect("missing deferral");
    assert_eq!(tf.response.code, 450);
    assert_eq!(tf.response.content, "busy");
    Ok(())
}

#[tokio::test]
async fn disconnect_in_data_try_next() -> anyhow::Result<()> {
    let mut daemon =
        DaemonWithMaildir::start_with_env(vec![("KUMOD_TRY_NEXT_HOST_ON_TRANSPORT_ERROR", "1")])
            .await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams {
        recip: Some("not-me@two-hosts.example.com"),
        // Cause the sink to 451 disconnect us in mail from.
        // This is to verify that we handle this sort of error during
        // a pipeline send correctly.
        sender: Some("disconnect-in-data-no-421@example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    let response = MailGenParams {
        recip: Some("pick-me@two-hosts.example.com"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| {
                summary.get(&TransientFailure).copied().unwrap_or(0) > 0
                    && summary.get(&Delivery).copied().unwrap_or(0) > 0
            },
            Duration::from_secs(10),
        )
        .await;

    daemon.stop_both().await?;

    let logs = daemon.source.collect_logs().await?;

    // We want to see a TransientFailure for the first message, and a Delivery
    // for the second message, both with the same session id.
    let tf = logs
        .iter()
        .find(|record| record.kind == TransientFailure)
        .expect("at least one TransientFailure");
    let delivery = logs
        .iter()
        .find(|record| record.kind == Delivery)
        .expect("at least one Delivery");

    // Same session, but different hosts is what we expect: it indicates
    // that the first message snipped the connection and that the second
    // one used the next option
    k9::assert_equal!(tf.session_id, delivery.session_id);
    assert_ne!(tf.peer_address, delivery.peer_address);

    k9::snapshot!(
        &tf.response,
        r#"
Response {
    code: 451,
    enhanced_code: None,
    content: "disconnecting disconnect-in-data-no-421",
    command: Some(
        ".\r
",
    ),
}
"#
    );

    assert_eq!(
        tf.peer_address,
        Some(ResolvedAddress {
            name: "localhost-1".to_string(),
            addr: daemon.sink.listener("smtp").into(),
            is_secure: false,
        })
    );
    assert_eq!(
        delivery.peer_address,
        Some(ResolvedAddress {
            name: "localhost-2".to_string(),
            addr: daemon.sink.listener("smtp").into(),
            is_secure: false,
        })
    );

    Ok(())
}
