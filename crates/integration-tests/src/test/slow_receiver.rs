//! Tests for delivering a large message to a receiver that accepts the
//! message data slowly, or stops accepting it altogether.
//!
//! The receiving side is a minimal SMTP server implemented here, because
//! the kumod sink always consumes data as fast as it arrives.
use crate::kumod::{DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use kumo_log_types::RecordType::{Delayed, Delivery, TransientFailure};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

/// The message needs to be much larger than the amount of data that the
/// kernel will buffer on behalf of a receiver that isn't reading (which can
/// be several MiB on loopback), or else the sender never observes the
/// receiver being slow.
const MESSAGE_SIZE: usize = 12 * 1024 * 1024;

/// How the receiver treats the message data it is sent
#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Accept data at no more than this many bytes per second, regardless
    /// of how much the kernel hands us on each read
    Throttle { bytes_per_sec: usize },
    /// Read as fast as possible until at least this many bytes have been
    /// received, then stop reading but leave the connection open.
    StallAfter(usize),
}

#[derive(Debug, Default)]
struct Stats {
    /// Bytes of message data received
    data_bytes: usize,
    /// How long it took to receive the most recently completed message
    /// data, from the 354 until the terminating dot.
    last_transfer: Option<Duration>,
    /// Number of messages that were completely received and acknowledged
    completed: usize,
}

struct SlowSink {
    addr: SocketAddr,
    stats: Arc<parking_lot::Mutex<Stats>>,
}

impl SlowSink {
    async fn start(mode: Mode) -> anyhow::Result<Self> {
        let socket = TcpSocket::new_v4()?;
        // A small receive buffer, set before listening so that it is
        // inherited by accepted connections, keeps the amount of data
        // that the kernel will absorb on our behalf small relative to the
        // size of the messages used here, so that a slow reader exerts
        // backpressure on the sender. It can't be too small though, as
        // tiny windows make TCP itself crawl.
        socket.set_recv_buffer_size(64 * 1024)?;
        socket.bind("127.0.0.1:0".parse()?)?;
        let listener: TcpListener = socket.listen(16)?;
        let addr = listener.local_addr()?;
        let stats = Arc::new(parking_lot::Mutex::new(Stats::default()));

        tokio::spawn({
            let stats = stats.clone();
            async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let stats = stats.clone();
                    tokio::spawn(async move {
                        if let Err(err) = Self::session(stream, mode, stats).await {
                            eprintln!("slow sink session ended: {err:#}");
                        }
                    });
                }
            }
        });

        Ok(Self { addr, stats })
    }

    async fn session(
        stream: TcpStream,
        mode: Mode,
        stats: Arc<parking_lot::Mutex<Stats>>,
    ) -> anyhow::Result<()> {
        let mut stream = BufReader::new(stream);
        stream.write_all(b"220 slow.example.com ESMTP\r\n").await?;

        let mut line = String::new();
        loop {
            line.clear();
            if stream.read_line(&mut line).await? == 0 {
                return Ok(());
            }
            let verb = line
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_uppercase();
            match verb.as_str() {
                "EHLO" | "HELO" => {
                    stream.write_all(b"250 slow.example.com\r\n").await?;
                }
                "MAIL" | "RCPT" | "RSET" | "NOOP" => {
                    stream.write_all(b"250 OK\r\n").await?;
                }
                "QUIT" => {
                    stream.write_all(b"221 bye\r\n").await?;
                    return Ok(());
                }
                "DATA" => {
                    stream.write_all(b"354 go ahead\r\n").await?;
                    Self::receive_data(&mut stream, mode, &stats).await?;
                    stream.write_all(b"250 accepted\r\n").await?;
                }
                _ => {
                    stream.write_all(b"502 not implemented\r\n").await?;
                }
            }
        }
    }

    async fn receive_data(
        stream: &mut BufReader<TcpStream>,
        mode: Mode,
        stats: &Arc<parking_lot::Mutex<Stats>>,
    ) -> anyhow::Result<()> {
        const END: &[u8] = b"\r\n.\r\n";
        let started = Instant::now();
        let mut buf = vec![0u8; 64 * 1024];
        let mut tail: Vec<u8> = vec![];
        let mut total = 0usize;

        loop {
            if let Mode::StallAfter(n) = mode {
                if total >= n {
                    // Stop reading, but keep the connection open
                    std::future::pending::<()>().await;
                }
            }
            let n = stream.read(&mut buf).await?;
            anyhow::ensure!(n > 0, "connection closed during DATA");
            total += n;
            stats.lock().data_bytes += n;
            if let Mode::Throttle { bytes_per_sec } = mode {
                tokio::time::sleep(Duration::from_secs_f64(n as f64 / bytes_per_sec as f64)).await;
            }

            tail.extend_from_slice(&buf[..n]);
            if tail.windows(END.len()).any(|w| w == END) {
                let mut stats = stats.lock();
                stats.last_transfer.replace(started.elapsed());
                stats.completed += 1;
                return Ok(());
            }
            // Only the last few bytes are needed to spot a terminator
            // that is split across reads
            if tail.len() > END.len() {
                tail.drain(..tail.len() - END.len());
            }
        }
    }
}

/// A large message to a receiver that accepts data slowly, such that the
/// transfer takes much longer than both data_block_timeout and the progress
/// watchdog threshold, must be delivered: each block is accepted in time,
/// and writing the data counts as forward progress for the watchdog.
#[tokio::test]
async fn slow_receiver_large_message_delivers() -> anyhow::Result<()> {
    // At this rate each 64KiB block is accepted in a small fraction of the
    // 2s block timeout, but the message is large enough that the transfer
    // takes longer than the 4s watchdog threshold, even allowing for the
    // several MiB of data that the kernel buffers on our behalf. That means
    // that the dispatcher is only kept alive because writing message data
    // is reported as forward progress.
    let sink = SlowSink::start(Mode::Throttle {
        bytes_per_sec: 2 * 1024 * 1024,
    })
    .await?;

    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_SLOW_SINK_PORT", sink.addr.port().to_string())
        .env("KUMOD_DATA_BLOCK_TIMEOUT", "2s")
        .env("KUMOD_WATCHDOG_TIMEOUT", "4s")
        .start()
        .await
        .context("DaemonWithMaildirOptions::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams {
        recip: Some("slow@example.com"),
        size: Some(MESSAGE_SIZE),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250, "unexpected response: {response:?}");

    let delivered = daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) >= 1,
            Duration::from_secs(120),
        )
        .await;
    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    anyhow::ensure!(
        delivered,
        "message was not delivered; source logs: {:#?}",
        logs.iter()
            .map(|r| format!("{:?}: {}", r.kind, r.response.to_single_line()))
            .collect::<Vec<_>>()
    );
    assert!(
        !logs
            .iter()
            .any(|r| r.kind == Delayed || r.kind == TransientFailure),
        "unexpected failure: {:#?}",
        logs.iter()
            .map(|r| format!("{:?}: {}", r.kind, r.response.to_single_line()))
            .collect::<Vec<_>>()
    );

    let (completed, data_bytes, transfer) = {
        let stats = sink.stats.lock();
        (stats.completed, stats.data_bytes, stats.last_transfer)
    };
    assert_eq!(completed, 1);
    assert!(data_bytes >= MESSAGE_SIZE, "data_bytes={data_bytes}");
    // Make sure that the test exercised what it set out to: a transfer
    // that took longer than both the block timeout and the watchdog.
    let transfer = transfer.context("no transfer time recorded")?;
    assert!(
        transfer > Duration::from_secs(5),
        "transfer took only {transfer:?}, which doesn't demonstrate anything"
    );

    Ok(())
}

/// A receiver that stops accepting data must be detected after about
/// data_block_timeout, and not be mistaken for a wedged dispatcher.
#[tokio::test]
async fn stalled_receiver_times_out() -> anyhow::Result<()> {
    let sink = SlowSink::start(Mode::StallAfter(256 * 1024)).await?;

    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_SLOW_SINK_PORT", sink.addr.port().to_string())
        .env("KUMOD_DATA_BLOCK_TIMEOUT", "2s")
        .env("KUMOD_WATCHDOG_TIMEOUT", "60s")
        .start()
        .await
        .context("DaemonWithMaildirOptions::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let response = MailGenParams {
        recip: Some("stalled@example.com"),
        size: Some(MESSAGE_SIZE),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    anyhow::ensure!(response.code == 250, "unexpected response: {response:?}");

    let failed = daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) >= 1,
            Duration::from_secs(30),
        )
        .await;
    daemon.stop_both().await.context("stop_both")?;

    let logs = daemon.source.collect_logs().await?;
    anyhow::ensure!(failed, "no transient failure was logged: {logs:#?}");

    let failure = logs
        .iter()
        .find(|r| r.kind == TransientFailure)
        .expect("checked above");
    let text = failure.response.to_single_line();
    assert!(
        text.contains("Timed Out sending message payload data"),
        "unexpected failure: {text}"
    );
    assert!(
        !logs.iter().any(|r| r.kind == Delivery),
        "should not have been delivered"
    );
    assert!(
        !logs
            .iter()
            .any(|r| r.response.to_single_line().contains("watchdog")),
        "the watchdog should not have been involved"
    );
    assert_eq!(sink.stats.lock().completed, 0);

    Ok(())
}
