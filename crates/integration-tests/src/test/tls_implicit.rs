use crate::kumod::{DaemonWithMaildir, MailGenParams};
use bstr::ByteSlice;
use k9::assert_equal;
use kumo_log_types::RecordType::{Delivery, Reception};
use rfc5321::TlsOptions;
use serde_json::Value;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

/// Verifies end-to-end delivery over implicit TLS (SMTPS): the sink listener
/// and the source egress path both run with `tls_mode = ImplicitTls` instead of
/// STARTTLS. The sink presents a self-signed certificate. The source keeps the
/// harness's `OpportunisticInsecure` `enable_tls` (source.lua) so that
/// certificate is accepted without validation.
#[tokio::test]
async fn tls_implicit() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_TEST_SINK_TLS_MODE", "ImplicitTls"),
        ("KUMOD_EGRESS_TLS_MODE", "ImplicitTls"),
    ])
    .await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await?;

    // The source delivered over an encrypted channel.
    let source_logs = daemon.source.collect_logs().await?;
    let delivery = source_logs
        .iter()
        .find(|record| record.kind == Delivery)
        .unwrap();
    assert!(delivery.tls_cipher.is_some());

    // Confirm the sink recognized the session as implicit TLS rather than as
    // a plaintext connection that happened to upgrade via STARTTLS.
    let sink_logs = daemon.sink.collect_logs().await?;
    let reception = sink_logs
        .iter()
        .find(|record| record.kind == Reception)
        .unwrap();
    assert_equal!(delivery.tls_cipher, reception.tls_cipher);
    assert_equal!(reception.meta.get("tls_implicit"), Some(&Value::Bool(true)));

    // The received message is traced as ESMTPS, the same as a STARTTLS session.
    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;
    let trace = parsed
        .headers()
        .get_first("Received")
        .unwrap()
        .as_unstructured()
        .unwrap();
    assert!(trace.contains_str(&format!(
        "with ESMTPS ({}:{})",
        reception.tls_protocol_version.as_ref().unwrap(),
        reception.tls_cipher.as_ref().unwrap()
    )));

    Ok(())
}

/// Sends `prefix` and the first buffer written to the stream in a single
/// write, as a load balancer does with its PROXY header when the client's
/// first bytes are already there.
struct PrefixFirstWrite<S> {
    prefix: Option<Vec<u8>>,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixFirstWrite<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixFirstWrite<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = &mut *self;
        let Some(prefix) = &this.prefix else {
            return Pin::new(&mut this.inner).poll_write(cx, buf);
        };
        let combined = [&prefix[..], buf].concat();
        match Pin::new(&mut this.inner).poll_write(cx, &combined) {
            Poll::Ready(Ok(written)) if written == combined.len() => {
                this.prefix = None;
                Poll::Ready(Ok(buf.len()))
            }
            Poll::Ready(Ok(written)) => Poll::Ready(Err(std::io::Error::other(format!(
                "short write: {written} of {} bytes",
                combined.len()
            )))),
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// PROXY v2, TCP over IPv4, 203.0.113.7:40000 -> 192.0.2.1:465
fn proxy_v2_header() -> Vec<u8> {
    let mut header = b"\r\n\r\n\0\r\nQUIT\n\x21\x11\x00\x0c".to_vec();
    header.extend_from_slice(&[203, 0, 113, 7, 192, 0, 2, 1]);
    header.extend_from_slice(&40000u16.to_be_bytes());
    header.extend_from_slice(&465u16.to_be_bytes());
    header
}

async fn start_implicit_tls_behind_proxy() -> anyhow::Result<DaemonWithMaildir> {
    DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_TEST_SINK_TLS_MODE", "ImplicitTls"),
        ("KUMOD_TEST_REQUIRE_PROXY_PROTOCOL", "1"),
    ])
    .await
}

/// Behind a load balancer, the PROXY header and the start of the client's
/// ClientHello often arrive together. The listener must take the header and
/// leave the rest to the TLS handshake.
#[tokio::test]
async fn tls_implicit_proxy_header_and_client_hello_in_one_write() -> anyhow::Result<()> {
    let mut daemon = start_implicit_tls_behind_proxy().await?;

    let stream = TcpStream::connect(daemon.sink.listener("smtp")).await?;
    let connector = TlsOptions {
        insecure: true,
        ..Default::default()
    }
    .build_tls_connector()
    .await?;
    let mut tls = tokio::time::timeout(
        Duration::from_secs(10),
        connector.connect(
            "localhost".try_into()?,
            PrefixFirstWrite {
                prefix: Some(proxy_v2_header()),
                inner: stream,
            },
        ),
    )
    .await??;

    let mut banner = vec![0u8; 512];
    let len = tokio::time::timeout(Duration::from_secs(10), tls.read(&mut banner)).await??;
    let banner = String::from_utf8_lossy(&banner[..len]);
    assert!(banner.starts_with("220 "), "{banner}");

    daemon.stop_both().await?;
    Ok(())
}

/// When a whole TLS record arrives with the PROXY header, the server must
/// process it instead of waiting for more from the client. The record here
/// holds an empty, invalid ClientHello: the server answers with an alert.
#[tokio::test]
async fn tls_implicit_proxy_header_and_whole_tls_record_in_one_write() -> anyhow::Result<()> {
    let mut daemon = start_implicit_tls_behind_proxy().await?;

    let mut stream = TcpStream::connect(daemon.sink.listener("smtp")).await?;
    let record = b"\x16\x03\x01\x00\x04\x01\x00\x00\x00";
    stream
        .write_all(&[&proxy_v2_header()[..], &record[..]].concat())
        .await?;

    let mut reply = [0u8; 1];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut reply)).await??;
    // 0x15: a TLS alert record.
    assert_eq!(reply[0], 0x15);

    daemon.stop_both().await?;
    Ok(())
}
