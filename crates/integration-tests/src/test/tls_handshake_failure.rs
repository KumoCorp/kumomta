use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::{Delivery, TransientFailure};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

async fn receive_starttls(stream: TcpStream) -> anyhow::Result<BufReader<TcpStream>> {
    let mut peer = BufReader::new(stream);
    peer.write_all(b"220 fallback.example.com\r\n").await?;
    let mut command = String::new();
    peer.read_line(&mut command).await?;
    anyhow::ensure!(command.starts_with("EHLO "), "{command:?}");
    peer.write_all(b"250-fallback.example.com\r\n250 STARTTLS\r\n")
        .await?;
    command.clear();
    peer.read_line(&mut command).await?;
    anyhow::ensure!(command == "STARTTLS\r\n", "{command:?}");
    Ok(peer)
}

/// Fail STARTTLS but leave TCP open: the client must close rather than send
/// plaintext SMTP commands on the socket that entered TLS negotiation.
async fn fail_handshake_and_expect_close(stream: TcpStream) -> anyhow::Result<()> {
    let mut peer = receive_starttls(stream).await?;
    peer.write_all(b"220 Ready for TLS\r\n").await?;
    let mut header = [0; 5];
    peer.read_exact(&mut header).await?;
    anyhow::ensure!(
        header[0] == 22,
        "expected a TLS handshake record: {header:?}"
    );
    let mut hello = vec![0; u16::from_be_bytes([header[3], header[4]]) as usize];
    peer.read_exact(&mut hello).await?;
    anyhow::ensure!(hello.first() == Some(&1), "expected ClientHello");
    // Fatal handshake_failure alert, not a timeout or a disconnected socket.
    peer.write_all(&[21, 3, 3, 0, 2, 2, 40]).await?;
    let mut after = [0; 256];
    let count = peer.read(&mut after).await?;
    anyhow::ensure!(count == 0, "bytes after failed TLS: {:?}", &after[..count]);
    Ok(())
}

async fn handshake_failure_reconnect(prefer_openssl: bool) -> anyhow::Result<()> {
    for (tls, reconnect, auth, expect_delivery) in [
        ("OpportunisticInsecure", false, false, true),
        ("OpportunisticInsecure", true, false, true),
        ("Opportunistic", false, false, false),
        ("Opportunistic", true, false, true),
        ("Required", true, false, false),
        ("RequiredInsecure", true, false, false),
        ("OpportunisticInsecure", false, true, false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut options = DaemonWithMaildirOptions::new()
            .env("KUMOD_ENABLE_TLS", tls)
            .env(
                "KUMOD_TEST_SMTP_PEER_PORT",
                listener.local_addr()?.port().to_string(),
            )
            .env("KUMOD_RETRY_INTERVAL", "1h");
        if prefer_openssl {
            options = options.env("KUMOD_PREFER_OPENSSL", "1");
        }
        if reconnect {
            options = options.env("KUMOD_OPPORTUNISTIC_TLS_RECONNECT", "1");
        }
        if auth {
            options = options
                .env("KUMOD_SMTP_AUTH_USERNAME", "interop-user")
                .env("KUMOD_SMTP_AUTH_PASSWORD", "synthetic-password");
        }
        let mut daemon = options.start().await?;
        let sink = daemon.sink.listener("smtp");
        let peer = async {
            let (stream, _) = listener.accept().await?;
            fail_handshake_and_expect_close(stream).await?;
            if expect_delivery || auth {
                // The real sink also advertises STARTTLS. The implicit retry
                // or explicit broken-TLS memory must skip it on a new connection.
                let (mut plain, _) = listener.accept().await?;
                let mut backend = TcpStream::connect(sink).await?;
                tokio::io::copy_bidirectional(&mut plain, &mut backend)
                    .await
                    .context("forward fresh fallback session")?;
            }
            Ok::<_, anyhow::Error>(())
        };
        let exercise = async {
            let mut client = daemon.smtp_client().await?;
            let response = MailGenParams::default().send(&mut client).await?;
            anyhow::ensure!(response.code == 250, "{response:?}");
            anyhow::ensure!(
                daemon
                    .wait_for_source_summary(
                        |s| s.get(&Delivery).copied().unwrap_or(0) > 0
                            || s.get(&TransientFailure).copied().unwrap_or(0) > 0,
                        Duration::from_secs(10),
                    )
                    .await,
                "no disposition for {tls}, reconnect={reconnect}"
            );
            // Keep the sink alive while the source closes the forwarded session.
            daemon.source.stop().await?;
            daemon.sink.stop().await?;
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(45), async {
            tokio::try_join!(peer, exercise)
        })
        .await?
        .with_context(|| format!("{tls}, reconnect={reconnect}, auth={auth}"))?;
        let records = daemon.source.collect_logs().await?;
        let outcome = records
            .iter()
            .find(|r| matches!(r.kind, Delivery | TransientFailure))
            .context("message disposition")?;
        anyhow::ensure!(
            outcome.kind
                == if expect_delivery {
                    Delivery
                } else {
                    TransientFailure
                },
            "{tls}, reconnect={reconnect}: {outcome:?}"
        );
        anyhow::ensure!(outcome.tls_cipher.is_none());
        if auth {
            anyhow::ensure!(
                outcome.response.content.contains("AUTH PLAIN is required"),
                "{outcome:?}"
            );
        }
        anyhow::ensure!(daemon.extract_maildir_messages()?.len() == usize::from(expect_delivery));
    }
    Ok(())
}

/// A failed plaintext retry must not loop, and must not make the next MX skip TLS.
async fn handshake_failure_retry_is_local(prefer_openssl: bool) -> anyhow::Result<()> {
    for deliver in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut options = DaemonWithMaildirOptions::new()
            .env("KUMOD_ENABLE_TLS", "OpportunisticInsecure")
            .env(
                "KUMOD_TEST_SMTP_PEER_PORT",
                listener.local_addr()?.port().to_string(),
            )
            .env("KUMOD_RETRY_INTERVAL", "1h");
        if prefer_openssl {
            options = options.env("KUMOD_PREFER_OPENSSL", "1");
        }
        let mut daemon = options.start().await?;
        let sink = daemon.sink.listener("smtp");
        let peer = async {
            for last in [false, true] {
                let (stream, _) = listener.accept().await?;
                // Both MX candidates must try TLS despite the previous candidate's failure.
                fail_handshake_and_expect_close(stream).await?;
                let (mut plain, _) = listener.accept().await?;
                if last && deliver {
                    let mut backend = TcpStream::connect(sink).await?;
                    tokio::io::copy_bidirectional(&mut plain, &mut backend).await?;
                } else {
                    plain.write_all(b"421 fallback unavailable\r\n").await?;
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        let exercise = async {
            let mut client = daemon.smtp_client().await?;
            let response = MailGenParams {
                recip: Some("recip@two-hosts.example.com"),
                ..Default::default()
            }
            .send(&mut client)
            .await?;
            anyhow::ensure!(response.code == 250, "{response:?}");
            anyhow::ensure!(
                daemon
                    .wait_for_source_summary(
                        |s| s.get(&Delivery).copied().unwrap_or(0) > 0
                            || s.get(&TransientFailure).copied().unwrap_or(0) > 0,
                        Duration::from_secs(10),
                    )
                    .await
            );
            daemon.source.stop().await?;
            daemon.sink.stop().await?;
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(45), async {
            tokio::try_join!(peer, exercise)
        })
        .await??;
        let records = daemon.source.collect_logs().await?;
        let outcome = records
            .iter()
            .find(|r| matches!(r.kind, Delivery | TransientFailure))
            .context("message disposition")?;
        anyhow::ensure!(
            outcome.kind == if deliver { Delivery } else { TransientFailure },
            "{outcome:?}"
        );
        anyhow::ensure!(outcome.tls_cipher.is_none());
        anyhow::ensure!(daemon.extract_maildir_messages()?.len() == usize::from(deliver));
    }
    Ok(())
}

/// STARTTLS rejection and timeout are errors, not FailedHandshake results eligible for fallback.
async fn starttls_errors_do_not_retry(prefer_openssl: bool) -> anyhow::Result<()> {
    for reject in [false, true] {
        for reconnect in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let mut options = DaemonWithMaildirOptions::new()
                .env("KUMOD_ENABLE_TLS", "OpportunisticInsecure")
                .env(
                    "KUMOD_TEST_SMTP_PEER_PORT",
                    listener.local_addr()?.port().to_string(),
                )
                .env("KUMOD_RETRY_INTERVAL", "1h");
            if prefer_openssl {
                options = options.env("KUMOD_PREFER_OPENSSL", "1");
            }
            if reconnect {
                options = options.env("KUMOD_OPPORTUNISTIC_TLS_RECONNECT", "1");
            }
            let mut daemon = options.start().await?;
            let peer = async {
                let (stream, _) = listener.accept().await?;
                let mut peer = receive_starttls(stream).await?;
                if reject {
                    peer.write_all(b"454 TLS unavailable\r\n").await?;
                } else {
                    peer.write_all(b"220 Ready for TLS\r\n").await?;
                    let mut header = [0; 5];
                    peer.read_exact(&mut header).await?;
                    let mut hello = vec![0; u16::from_be_bytes([header[3], header[4]]) as usize];
                    peer.read_exact(&mut hello).await?;
                    // Hold the socket open without responding until STARTTLS times out.
                }
                anyhow::ensure!(peer.read(&mut [0; 1]).await? == 0);
                Ok::<_, anyhow::Error>(())
            };
            let exercise = async {
                let mut client = daemon.smtp_client().await?;
                anyhow::ensure!(MailGenParams::default().send(&mut client).await?.code == 250);
                anyhow::ensure!(
                    daemon
                        .wait_for_source_summary(
                            |s| s.get(&TransientFailure).copied().unwrap_or(0) > 0,
                            Duration::from_secs(15),
                        )
                        .await
                );
                daemon.source.stop().await?;
                daemon.sink.stop().await?;
                Ok::<_, anyhow::Error>(())
            };
            tokio::time::timeout(Duration::from_secs(45), async {
                tokio::try_join!(peer, exercise)
            })
            .await?
            .with_context(|| format!("reject={reject}, reconnect={reconnect}"))?;
            // The listener stays bound throughout the test so an unwanted retry is observable.
            anyhow::ensure!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
            anyhow::ensure!(daemon.extract_maildir_messages()?.is_empty());
        }
    }
    Ok(())
}

#[tokio::test]
async fn tls_handshake_failure_retry_is_local_openssl() -> anyhow::Result<()> {
    handshake_failure_retry_is_local(true).await
}

#[tokio::test]
async fn tls_handshake_failure_retry_is_local_rustls() -> anyhow::Result<()> {
    handshake_failure_retry_is_local(false).await
}

#[tokio::test]
async fn tls_starttls_errors_do_not_retry_openssl() -> anyhow::Result<()> {
    starttls_errors_do_not_retry(true).await
}

#[tokio::test]
async fn tls_starttls_errors_do_not_retry_rustls() -> anyhow::Result<()> {
    starttls_errors_do_not_retry(false).await
}

/// A successful TLS handshake followed by failed EHLO must not leave TLS
/// metadata on a later plaintext connection in the same dispatcher session.
async fn reconnect_clears_tls_info(prefer_openssl: bool) -> anyhow::Result<()> {
    use rfc5321::client::tokio_rustls::{rustls, TlsAcceptor};
    use std::sync::Arc;

    let key = rcgen::KeyPair::generate()?;
    let cert = rcgen::CertificateParams::new(vec!["fallback.example.com".to_string()])?
        .self_signed(&key)?;
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    for reconnect in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut options = DaemonWithMaildirOptions::new()
            .env("KUMOD_ENABLE_TLS", "OpportunisticInsecure")
            .env(
                "KUMOD_TEST_SMTP_PEER_PORT",
                listener.local_addr()?.port().to_string(),
            )
            .env("KUMOD_RETRY_INTERVAL", "1h");
        if prefer_openssl {
            options = options.env("KUMOD_PREFER_OPENSSL", "1");
        }
        if reconnect {
            options = options.env("KUMOD_OPPORTUNISTIC_TLS_RECONNECT", "1");
        }
        let mut daemon = options.start().await?;
        let sink = daemon.sink.listener("smtp");
        let peer = async {
            let (stream, _) = listener.accept().await?;
            let mut peer = receive_starttls(stream).await?;
            peer.write_all(b"220 Ready for TLS\r\n").await?;
            let mut tls = BufReader::new(acceptor.accept(peer.into_inner()).await?);
            let mut command = String::new();
            tls.read_line(&mut command).await?;
            anyhow::ensure!(command.starts_with("EHLO "), "{command:?}");
            tls.write_all(b"421 Post-handshake EHLO rejected\r\n")
                .await?;
            drop(tls);

            if !reconnect {
                // Without explicit reconnect, EHLO failure moves to the next MX.
                // Its handshake failure then exercises the implicit plaintext retry.
                let (stream, _) = listener.accept().await?;
                fail_handshake_and_expect_close(stream).await?;
            }
            let (mut plain, _) = listener.accept().await?;
            let mut backend = TcpStream::connect(sink).await?;
            tokio::io::copy_bidirectional(&mut plain, &mut backend).await?;
            Ok::<_, anyhow::Error>(())
        };
        let exercise = async {
            let mut client = daemon.smtp_client().await?;
            let response = MailGenParams {
                recip: Some("recip@two-hosts.example.com"),
                ..Default::default()
            }
            .send(&mut client)
            .await?;
            anyhow::ensure!(response.code == 250, "{response:?}");
            anyhow::ensure!(
                daemon
                    .wait_for_source_summary(
                        |s| s.get(&Delivery).copied().unwrap_or(0) > 0,
                        Duration::from_secs(10),
                    )
                    .await
            );
            daemon.source.stop().await?;
            daemon.sink.stop().await?;
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(45), async {
            tokio::try_join!(peer, exercise)
        })
        .await?
        .with_context(|| format!("reconnect={reconnect}"))?;
        let records = daemon.source.collect_logs().await?;
        let delivery = records
            .iter()
            .find(|r| r.kind == Delivery)
            .context("plaintext delivery")?;
        anyhow::ensure!(delivery.tls_cipher.is_none(), "{delivery:?}");
        anyhow::ensure!(delivery.tls_protocol_version.is_none(), "{delivery:?}");
        anyhow::ensure!(delivery.tls_peer_subject_name.is_none(), "{delivery:?}");
        let records = daemon.sink.collect_logs().await?;
        let reception = records
            .iter()
            .find(|r| r.kind == kumo_log_types::RecordType::Reception)
            .context("plaintext reception")?;
        anyhow::ensure!(reception.tls_cipher.is_none(), "{reception:?}");
        anyhow::ensure!(daemon.extract_maildir_messages()?.len() == 1);
    }
    Ok(())
}

#[tokio::test]
async fn tls_reconnect_clears_tls_info_openssl() -> anyhow::Result<()> {
    reconnect_clears_tls_info(true).await
}

#[tokio::test]
async fn tls_reconnect_clears_tls_info_rustls() -> anyhow::Result<()> {
    reconnect_clears_tls_info(false).await
}

#[tokio::test]
async fn tls_handshake_failure_openssl() -> anyhow::Result<()> {
    handshake_failure_reconnect(true).await
}

#[tokio::test]
async fn tls_handshake_failure_rustls() -> anyhow::Result<()> {
    handshake_failure_reconnect(false).await
}
