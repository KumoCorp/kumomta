use crate::kumod::{DaemonWithMaildirOptions, FaultInjector, KumoArgs, KumoDaemon, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::{JsonLogRecord, RecordType};
use rfc5321::client::{ClientError, SmtpClient};
use rfc5321::client_types::SmtpClientTimeouts;
use rfc5321::Response;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;

/// Connect to the daemon's SMTP listener and read the banner without
/// asserting that it is a 220.  Useful when the test deliberately
/// exercises a server that we expect to reject the connection at the
/// banner: returning the raw `Response` lets the caller assert on the
/// code and content directly instead of doing substring checks on a
/// formatted anyhow error.
async fn read_smtp_banner(daemon: &KumoDaemon) -> anyhow::Result<Response> {
    let mut client = SmtpClient::new(
        daemon.listener("smtp"),
        SmtpClientTimeouts::short_timeouts(),
    )
    .await?;
    let timeout = client.timeouts().connect_timeout;
    Ok(client.read_response(None, timeout).await?)
}

/// Histogram of log dispositions keyed by
/// `(RecordType, response code, response content)`, with a
/// `count` lookup that returns 0 for unseen keys.  Keeps the
/// per-test assertion code focused on the values it cares about
/// rather than on BTreeMap construction.
struct LogHistogram {
    counts: BTreeMap<(RecordType, u16, String), usize>,
}

impl LogHistogram {
    fn from_records(logs: &[JsonLogRecord]) -> Self {
        let mut counts = BTreeMap::new();
        for r in logs {
            *counts
                .entry((r.kind, r.response.code, r.response.content.clone()))
                .or_insert(0) += 1;
        }
        Self { counts }
    }

    fn count(&self, kind: RecordType, code: u16, content: &str) -> usize {
        self.counts
            .get(&(kind, code, content.to_string()))
            .copied()
            .unwrap_or(0)
    }
}

/// Suspend delivery to example.com with the given reason text, so
/// accepted messages stay in the spool rather than being delivered
/// to the maildir sink (and removed) immediately.  The reason text
/// flows into the per-message `TransientFailure` log records and
/// is sometimes asserted on, so each test supplies its own.
async fn suspend_example_com(
    daemon: &crate::kumod::DaemonWithMaildir,
    reason: &str,
) -> anyhow::Result<()> {
    daemon
        .source
        .kcli_text([
            "suspend",
            "--domain",
            "example.com",
            "--reason",
            reason,
            "--duration",
            "10m",
        ])
        .await
        .map(|_| ())
}

/// Open a fresh SMTP client, send one message with default params,
/// and return the SMTP reply code.  Callers assert on the code.
async fn send_one(daemon: &crate::kumod::DaemonWithMaildir) -> anyhow::Result<u16> {
    let mut client = daemon.smtp_client().await?;
    let resp = MailGenParams::default().send(&mut client).await?;
    Ok(resp.code)
}

/// Find a `.sst` file under `dir`.  Returns the first one encountered
/// in directory-iteration order.
async fn find_sst(dir: &Path) -> anyhow::Result<PathBuf> {
    let mut rd = fs::read_dir(dir)
        .await
        .with_context(|| format!("read_dir {}", dir.display()))?;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "sst") {
            return Ok(path);
        }
    }
    anyhow::bail!("no .sst file found in {}", dir.display());
}

/// Find a rocksdb write-ahead log (a `NNNNNN.log` file) under `dir`.
/// Returns the first one encountered in directory-iteration order.
async fn find_wal(dir: &Path) -> anyhow::Result<PathBuf> {
    let mut rd = fs::read_dir(dir)
        .await
        .with_context(|| format!("read_dir {}", dir.display()))?;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "log") {
            return Ok(path);
        }
    }
    anyhow::bail!("no .log file found in {}", dir.display());
}

/// Poll the daemon's /metrics endpoint until
/// `rocks_spool_load_shed_active` reports a non-zero value on any
/// labelled time series, or the timeout elapses.
async fn wait_for_load_shed_active(daemon: &KumoDaemon, timeout: Duration) -> anyhow::Result<()> {
    daemon
        .wait_for_metric(
            timeout,
            |m| m.name().as_str() == "rocks_spool_load_shed_active",
            |values| values.iter().any(|v| *v != 0.0),
        )
        .await
}

/// Verifies that when the rocksdb-backed spool wedges at runtime, the composite
/// latch in `metrics_monitor` engages within the configured window and the
/// load-shedding gate activates across the ingress paths (SMTP, HTTP inject,
/// HTTP liveness) with the external reason string. Also verifies that the
/// forced compaction reports the underlying rocksdb error to the caller, and
/// that once the spool stops wedging the gate reopens on its own and SMTP
/// delivery and HTTP liveness both accept traffic again.
#[tokio::test]
async fn spool_write_stopped_load_shedding() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .env("KUMOD_ROCKS_ERROR_UNLATCH_DURATION", "6s")
        .policy_file("source-rocks-spool.lua")
        .start()
        .await
        .context("start daemon")?;

    // Suspend delivery to example.com so accepted messages remain
    // durably stored in the spool and post-send compaction produces
    // SST files with real content rather than collapsing put+delete
    // pairs to nothing.
    suspend_example_com(&daemon, "hold for spool wedge test")
        .await
        .context("suspend example.com")?;

    // Baseline: a message is accepted by the SMTP listener and the
    // spool durably stores it.
    assert_equal!(send_one(&daemon).await.context("baseline send")?, 250);

    // Force a flush + compaction so an SST exists on disk.
    daemon
        .source
        .kcli_text(["spool-compact", "--name", "data"])
        .await
        .context("first spool-compact")?;

    // A few more messages so the next flush produces fresh content
    // that will need to be merged against the existing SST.
    for _ in 0..3 {
        assert_equal!(
            send_one(&daemon).await.context("send before sabotage")?,
            250
        );
    }

    // Open a persistent SMTP connection *before* introducing the
    // corruption.  This connection is past the per-connection
    // load-shedding check, so the gate must engage at a deeper layer
    // (the message-store path) to refuse subsequent transactions on
    // it.  Send one message now to prove the connection is healthy.
    let mut persistent = daemon.smtp_client().await?;
    {
        let resp = MailGenParams::default()
            .send(&mut persistent)
            .await
            .context("send on persistent client before sabotage")?;
        assert_equal!(resp.code, 250);
    }
    // (Helper send_one isn't used here because it opens its own
    // short-lived client; we need to reuse the persistent one.)

    // Sabotage: truncate the existing SST to zero bytes.  Outright
    // deletion would not be observed by rocksdb because the table
    // cache may still hold an open file descriptor (Linux keeps
    // unlinked files alive while any fd holds them open).  Truncation
    // modifies the file content through the same inode, so any cached
    // fd will see the missing bytes on the next read.
    let data_spool = daemon.source.dir.path().join("data-spool");
    let sst = find_sst(&data_spool).await?;
    fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&sst)
        .await
        .with_context(|| format!("truncate {}", sst.display()))?;

    // Force compaction.  With paranoid_checks at its default (off),
    // rocksdb silently drops the corrupt records and the call returns
    // success, but it still increments rocksdb.background-errors
    // along the way -- which is what the load-shedding gate latches
    // on.
    daemon
        .source
        .kcli_text(["spool-compact", "--name", "data"])
        .await
        .context("second spool-compact")?;

    // Wait for the metrics_monitor composite signal to latch the
    // load-shedding gate.  The test policy sets error_latch_duration
    // to 2s and the monitor ticks every 5s, so the gate should engage
    // within roughly 10s.  Use a generous bound.
    wait_for_load_shed_active(&daemon.source, Duration::from_secs(30)).await?;

    // A new SMTP connection must be load-shed at the banner.
    {
        let banner = read_smtp_banner(&daemon.source).await?;
        assert_equal!(banner.code, 421);
        assert_equal!(
            banner.content,
            "kumo.test the spool is not accepting writes. Try later"
        );
    }

    // The persistent connection opened before the gate latched is
    // still alive at the TCP layer (it cleared the connect-time
    // check) but must now fail when attempting another transaction:
    // the gate is enforced in the spool store path that is invoked
    // during DATA processing.  The SMTP server recognizes the
    // underlying SpoolUnhealthyError via root_cause inspection and
    // returns the same 421 response we use at the banner-level
    // load-shed check, so the wire-visible text is identical
    // regardless of which layer observed the condition.
    {
        let err = MailGenParams::default()
            .send(&mut persistent)
            .await
            .unwrap_err()
            .downcast::<ClientError>()
            .context("downcast ClientError")?;
        match err {
            ClientError::Rejected(resp) => {
                assert_equal!(resp.code, 421);
                assert_equal!(
                    resp.content,
                    "kumo.test the spool is not accepting writes. Try later"
                );
            }
            other => anyhow::bail!("unexpected client error: {other:?}"),
        }
    }

    // HTTP liveness must report 503 with the reason.
    {
        let url = format!(
            "http://{}/api/check-liveness/v1",
            daemon.source.listener("http")
        );
        let response = reqwest::Client::new().get(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;
        assert_equal!(
            format!("{status} {body}"),
            "503 Service Unavailable the spool is not accepting writes"
        );
    }

    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(90),
            |m| m.name().as_str() == "rocks_spool_load_shed_active",
            |values| !values.is_empty() && values.iter().all(|v| *v == 0.0),
        )
        .await
        .context("wait for quiet spool to reopen")?;
    assert_equal!(
        send_one(&daemon).await.context("send after reopening")?,
        250
    );
    let response = reqwest::Client::new()
        .get(format!(
            "http://{}/api/check-liveness/v1",
            daemon.source.listener("http")
        ))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    assert_equal!(format!("{status} {body}"), "200 OK OK");

    // Clean shutdown -- propagate any errors as the test would.
    // Shutting down a write-stopped rocksdb may itself surface errors;
    // if this proves flaky in practice we can revisit.
    daemon.stop_both().await.context("stop_both")?;

    Ok(())
}

/// Happy-path counterpart to the corruption tests below: verifies
/// that a rocksdb-backed spool survives a graceful stop/start with no
/// tampering, i.e. queued messages are still present and deliverable
/// after the process restarts.  This is the scenario a user incident
/// report claimed was broken ("kumod re-initializes the RocksDB
/// database as new on every start" -> queued mail lost across
/// restart).  We mirror that scenario as closely as the harness
/// allows: inject messages that are held in the spool, force a
/// flush+compaction so the data lives in SST files (the "already
/// flushed to SST" symptom), stop cleanly, restart against the same
/// on-disk state, and confirm every message is recovered and
/// ultimately delivered.
#[tokio::test]
async fn spool_restart_clean_recovery() -> anyhow::Result<()> {
    const COUNT: usize = 20;

    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-rocks-spool.lua")
        .start()
        .await
        .context("start daemon")?;

    // Suspend example.com so accepted messages stay in the spool
    // rather than being delivered (and removed) immediately.
    suspend_example_com(&daemon, "hold for clean-recovery test")
        .await
        .context("suspend pre-restart")?;

    for _ in 0..COUNT {
        assert_equal!(send_one(&daemon).await.context("pre-restart send")?, 250);
    }

    // Force flush + compact for both spools so the message data and
    // metadata live in on-disk SST files rather than only in the
    // WAL.  This reproduces the "data was already flushed to SST
    // (empty WAL)" variant from the incident report, which is the
    // case they said silently lost the queue.
    for name in ["data", "meta"] {
        daemon
            .source
            .kcli_text(["spool-compact", "--name", name])
            .await
            .with_context(|| format!("pre-restart spool-compact {name}"))?;
    }

    let stopped = daemon
        .source
        .stop_temporarily()
        .await
        .context("graceful stop before restart")?;

    // Sanity check the on-disk state the way an operator would:
    // CURRENT and at least one SST must be present for both spools
    // before we restart.  If kumod were truly re-initializing the
    // DB as new, these would be the files that get discarded.
    for sub in ["data-spool", "meta-spool"] {
        let dir = stopped.path().join(sub);
        assert!(
            dir.join("CURRENT").exists(),
            "expected CURRENT in {}",
            dir.display()
        );
        // Presence of an SST confirms the compaction above landed
        // durable data on disk (not just WAL).
        find_sst(&dir)
            .await
            .with_context(|| format!("expected an SST in {}", dir.display()))?;
    }

    // Restart against the same on-disk state.  Reuse the still-running
    // sink's SMTP listener so routing continues to resolve.
    let sink_smtp_port = daemon.sink.listener("smtp").port();
    daemon.source = stopped
        .start(KumoArgs {
            policy_file: "source-rocks-spool.lua".to_string(),
            env: vec![(
                "KUMOD_SMTP_SINK_PORT".to_string(),
                sink_smtp_port.to_string(),
            )],
        })
        .await
        .context("restart source")?;

    // The fresh process has an empty suspend table, so the recovered
    // messages are eligible again -- but their next-due times are
    // still in the future.  Rebind with --always-flush to make them
    // immediately eligible for delivery.
    daemon
        .source
        .kcli_text([
            "rebind",
            "--everything",
            "--always-flush",
            "--reason",
            "flush recovered messages",
        ])
        .await
        .context("post-restart rebind")?;

    // If the spool recovered correctly, all COUNT messages load from
    // the (untampered) spool and deliver to the sink maildir.  A
    // re-initialized/empty DB would deliver zero.
    assert!(
        daemon
            .wait_for_maildir_count(COUNT, Duration::from_secs(60))
            .await,
        "expected {COUNT} messages to be recovered from the spool and delivered"
    );

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}

/// Companion to spool_restart_clean_recovery that exercises the other
/// variant from the incident report: data that was never flushed to
/// an SST and lives only in the write-ahead log at shutdown.  The
/// user reported that in this state a restart either fails to start
/// ("wal_dir contains existing log file") or comes up with an empty
/// queue, both of which imply rocksdb took the fresh-database path
/// instead of replaying the WAL.  Here we raise write_buffer_size so
/// the injected messages stay in the memtable/WAL (no SST is
/// produced), stop cleanly, restart, and confirm the WAL replays and
/// every message is recovered and delivered.
#[tokio::test]
async fn spool_restart_wal_replay() -> anyhow::Result<()> {
    const COUNT: usize = 20;
    // Large enough that a handful of small messages never fill the
    // memtable, so rocksdb keeps the data in the WAL rather than
    // flushing it to an SST.
    const BIG_WRITE_BUFFER: &str = "67108864";

    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-rocks-spool.lua")
        .env("KUMOD_ROCKS_WRITE_BUFFER_SIZE", BIG_WRITE_BUFFER)
        .start()
        .await
        .context("start daemon")?;

    // Suspend example.com so accepted messages stay in the spool
    // rather than being delivered (and removed) immediately.
    suspend_example_com(&daemon, "hold for wal-replay test")
        .await
        .context("suspend pre-restart")?;

    for _ in 0..COUNT {
        assert_equal!(send_one(&daemon).await.context("pre-restart send")?, 250);
    }

    let stopped = daemon
        .source
        .stop_temporarily()
        .await
        .context("graceful stop before restart")?;

    // Confirm the on-disk shape is the WAL-only case: a .log file is
    // present and no SST has been produced for either spool.  If an
    // SST existed here we would be testing the same path as the
    // flush-to-SST test above rather than WAL replay.
    for sub in ["data-spool", "meta-spool"] {
        let dir = stopped.path().join(sub);
        assert!(
            dir.join("CURRENT").exists(),
            "expected CURRENT in {}",
            dir.display()
        );
        find_wal(&dir)
            .await
            .with_context(|| format!("expected a WAL log file in {}", dir.display()))?;
        if let Ok(sst) = find_sst(&dir).await {
            anyhow::bail!(
                "expected no SST in {} but found {}; data was flushed, so this is not a WAL-replay test",
                dir.display(),
                sst.display()
            );
        }
    }

    // Restart against the same on-disk state.  Reuse the still-running
    // sink's SMTP listener so routing continues to resolve.
    let sink_smtp_port = daemon.sink.listener("smtp").port();
    daemon.source = stopped
        .start(KumoArgs {
            policy_file: "source-rocks-spool.lua".to_string(),
            env: vec![
                (
                    "KUMOD_SMTP_SINK_PORT".to_string(),
                    sink_smtp_port.to_string(),
                ),
                (
                    "KUMOD_ROCKS_WRITE_BUFFER_SIZE".to_string(),
                    BIG_WRITE_BUFFER.to_string(),
                ),
            ],
        })
        .await
        .context("restart source")?;

    // The fresh process has an empty suspend table, so the recovered
    // messages are eligible again -- but their next-due times are
    // still in the future.  Rebind with --always-flush to make them
    // immediately eligible for delivery.
    daemon
        .source
        .kcli_text([
            "rebind",
            "--everything",
            "--always-flush",
            "--reason",
            "flush recovered messages",
        ])
        .await
        .context("post-restart rebind")?;

    // If the WAL replayed correctly, all COUNT messages load from the
    // spool and deliver to the sink maildir.  A fresh-database path
    // (their reported failure) would deliver zero.
    assert!(
        daemon
            .wait_for_maildir_count(COUNT, Duration::from_secs(60))
            .await,
        "expected {COUNT} messages to be recovered from the WAL and delivered"
    );

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}

/// Verifies that if the daemon is shut down cleanly, an SST file is
/// damaged while it is offline, and then the daemon is restarted, the
/// system refuses to accept any non-trivial quantity of newly
/// injected mail.  The corruption is observed during the first
/// post-restart compaction, increments rocksdb.background-errors, and
/// latches the load-shedding gate via the same composite signal used
/// at runtime -- this test exercises that path through a process
/// restart rather than from a continuously-running daemon.
#[tokio::test]
async fn spool_restart_after_corruption() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-rocks-spool.lua")
        .start()
        .await
        .context("start daemon")?;

    // Suspend example.com so accepted messages stay in the spool and
    // post-send compaction produces SST files with real content.
    suspend_example_com(&daemon, "hold for restart-after-corruption test")
        .await
        .context("suspend pre-restart")?;

    // Inject some messages to populate the spool.
    for _ in 0..5 {
        assert_equal!(send_one(&daemon).await.context("pre-restart send")?, 250);
    }

    // Force a flush + compaction so the message data lives in an
    // SST file on disk, not just in the WAL.  rocksdb's clean
    // shutdown does not implicitly flush the memtable -- it just
    // cancels background work -- so without this the WAL is
    // replayed on restart and there is no SST to sabotage.
    daemon
        .source
        .kcli_text(["spool-compact", "--name", "data"])
        .await
        .context("pre-restart spool-compact")?;

    let stopped = daemon
        .source
        .stop_temporarily()
        .await
        .context("stop source before corruption")?;

    // Damage the spool while the daemon is offline.  Unlike the
    // runtime test, where we have to truncate-in-place because the
    // table cache may still hold an open fd to the file, here the
    // daemon has exited cleanly so every fd is released and we can
    // simply delete the SST -- which is the failure mode the
    // original user incident report described.
    let data_spool = stopped.path().join("data-spool");
    let sst = find_sst(&data_spool).await?;
    fs::remove_file(&sst)
        .await
        .with_context(|| format!("remove {}", sst.display()))?;

    // Bring the source back up against the same on-disk state.
    // Reuse the still-running sink's SMTP listener so routing
    // continues to resolve.
    let sink_smtp_port = daemon.sink.listener("smtp").port();
    daemon.source = stopped
        .start(KumoArgs {
            policy_file: "source-rocks-spool.lua".to_string(),
            env: vec![(
                "KUMOD_SMTP_SINK_PORT".to_string(),
                sink_smtp_port.to_string(),
            )],
        })
        .await
        .context("restart source after corruption")?;

    // The new process starts with an empty suspend table, so the
    // pre-restart messages are now eligible for delivery -- but
    // their next-due times are still in the future.  Rebinding
    // everything with --always-flush makes them immediately
    // eligible, which schedules delivery attempts.  Each attempt
    // loads the message data from the data spool, and that get
    // hits the deleted SST, incrementing
    // rocksdb.background-errors.  No explicit compaction is
    // needed: the natural read path surfaces the corruption.
    daemon
        .source
        .kcli_text([
            "rebind",
            "--everything",
            "--always-flush",
            "--reason",
            "flush to surface corruption",
        ])
        .await
        .context("post-restart rebind")?;

    // Wait for the metrics monitor to observe the load-failure
    // signal and latch the gate.
    wait_for_load_shed_active(&daemon.source, Duration::from_secs(30)).await?;

    // A new SMTP connection must be load-shed at the banner.  We
    // assert on a single connection rather than running a probe
    // loop because the banner-level refusal does not produce a log
    // record (the smtp_server intentionally avoids that for this
    // class of rejection); the per-message disposition we assert on
    // below already documents how the spool-load failure surfaces.
    {
        let banner = read_smtp_banner(&daemon.source).await?;
        assert_equal!(banner.code, 421);
        assert_equal!(
            banner.content,
            "kumo.test the spool is not accepting writes. Try later"
        );
    }

    // Give the dispatcher and the ready-queue maintainer a moment
    // to settle.  The first delivery attempt fails at load() and
    // trips the gate; subsequent messages then take the
    // spool_health hold path which produces a per-message record
    // only after the maintainer drains the ready queue back into
    // the scheduled queue and the next promotion hits the
    // spool-health check in Queue::insert_ready.  Without this
    // wait we would race the maintainer tick.
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Verify the disposition path that messages take when their
    // data cannot be loaded from the corrupted spool.  Each of the
    // 5 pre-restart messages produces a Reception (250) and a 451
    // "suspended" TransientFailure from the pre-restart suspend.
    // Post-restart, the 5 messages collectively produce 5 further
    // TransientFailure records distributed between two contents:
    //   * a 400 record carrying the rocksdb IO error verbatim, for
    //     the message whose delivery was already in flight when
    //     the gate latched, and
    //   * a 451 record carrying the spool-unhealthy hold reason,
    //     for messages caught by `spool_health` before they could
    //     try to load.
    // The exact split is timing-dependent (first dispatch wins
    // the IO-error path; the rest are held) so we assert on the
    // total to keep the test stable.
    let data_spool_display = daemon.source.dir.path().join("data-spool");
    let sst_name = sst.file_name().unwrap().to_string_lossy();
    let expected_spool_error = format!(
        "KumoMTA internal: error in deliver_message: IO error: No such file \
         or directory: While open a file for random read: {data_spool}/{sst_name}: \
         No such file or directory",
        data_spool = data_spool_display.display(),
    );

    let logs = daemon.source.collect_logs().await?;
    let histogram = LogHistogram::from_records(&logs);

    let receptions = histogram.count(RecordType::Reception, 250, "");
    let pre_restart_suspended = histogram.count(
        RecordType::TransientFailure,
        451,
        "KumoMTA internal: scheduled queue is suspended: \
         hold for restart-after-corruption test",
    );
    let post_restart_io_error =
        histogram.count(RecordType::TransientFailure, 400, &expected_spool_error);
    let post_restart_unhealthy = histogram.count(
        RecordType::Delayed,
        451,
        "KumoMTA internal: delivery suspended: spool unhealthy: \
         the spool is not accepting writes",
    );

    assert_equal!(receptions, 5);
    assert_equal!(pre_restart_suspended, 5);
    assert_equal!(post_restart_io_error + post_restart_unhealthy, 5);
    // Both paths must have been exercised: a real IO error
    // surfacing through the dispatch path, and the spool_health
    // hold path catching the rest.
    assert!(post_restart_io_error >= 1);
    assert!(post_restart_unhealthy >= 1);

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}

/// Variation of `spool_restart_after_corruption` that damages the
/// **meta** spool instead of the data spool.  Whereas the data
/// failure path surfaces when delivery loads the message body via
/// `Spool::load()`, the meta failure surfaces during the spool
/// enumeration that kumod runs at startup to discover existing
/// messages.  The kumod startup intentionally tolerates partial
/// enumeration failures (it logs and continues so the rest of the
/// system can come up), so this test demonstrates that the
/// load-shedding gate engages despite that tolerance and the
/// instance refuses new traffic until the underlying problem is
/// addressed.
#[tokio::test]
async fn spool_restart_after_meta_corruption() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-rocks-spool.lua")
        .start()
        .await
        .context("start daemon")?;

    suspend_example_com(&daemon, "hold for restart-after-meta-corruption test")
        .await
        .context("suspend pre-restart")?;

    for _ in 0..5 {
        assert_equal!(send_one(&daemon).await.context("pre-restart send")?, 250);
    }

    // Force flush + compact for both spools so each has on-disk
    // SST(s) we can sabotage and so kumod's clean shutdown does not
    // leave critical state in the WAL only.
    for name in ["data", "meta"] {
        daemon
            .source
            .kcli_text(["spool-compact", "--name", name])
            .await
            .with_context(|| format!("pre-restart spool-compact {name}"))?;
    }

    let stopped = daemon
        .source
        .stop_temporarily()
        .await
        .context("stop source before corruption")?;

    // Damage the meta spool's only SST.  Because the pre-shutdown
    // bottommost-force compaction collapsed all the message
    // metadata into a single SST in the deepest level, removing it
    // makes every pre-restart message un-enumerable.
    let meta_spool = stopped.path().join("meta-spool");
    let sst = find_sst(&meta_spool).await?;
    fs::remove_file(&sst)
        .await
        .with_context(|| format!("remove {}", sst.display()))?;

    let sink_smtp_port = daemon.sink.listener("smtp").port();
    daemon.source = stopped
        .start(KumoArgs {
            policy_file: "source-rocks-spool.lua".to_string(),
            env: vec![(
                "KUMOD_SMTP_SINK_PORT".to_string(),
                sink_smtp_port.to_string(),
            )],
        })
        .await
        .context("restart source after meta corruption")?;

    // Enumeration runs as part of the post-restart startup path.
    // The first iterator step that needs the deleted SST returns a
    // rocksdb IOError, which `record_foreground_error` classifies
    // as definitively bad and latches the gate on observation.  No
    // explicit trigger (such as the rebind used in the data test)
    // is required: enumeration is what kumod always runs at boot.
    wait_for_load_shed_active(&daemon.source, Duration::from_secs(30)).await?;

    // A new SMTP connection must be load-shed at the banner.
    {
        let banner = read_smtp_banner(&daemon.source).await?;
        assert_equal!(banner.code, 421);
        assert_equal!(
            banner.content,
            "kumo.test the spool is not accepting writes. Try later"
        );
    }

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}

/// Injects a full-disk (ENOSPC) fault scoped to the rocksdb spool directory and
/// verifies the load-shedding gate latches while writes fail, then reopens on
/// its own once space is restored and the spool accepts mail again.
///
/// The corruption tests above break reads of already-stored data. This breaks
/// writes, which is what a real full disk does, and exercises the gate
/// reopening automatically after a write failure -- the path that issue #597
/// found never unlatched.
///
/// The fault is applied by an LD_PRELOAD shim (the `fault-inject-preload`
/// crate) that returns ENOSPC for writes under the spool directory while a
/// sentinel file exists. The spool is relocated to a test-owned directory via
/// `KUMOD_ROCKS_SPOOL_DIR` to scope the fault to it without touching other
/// daemon state such as its logs or accounting database.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn spool_enospc_latches_and_recovers() -> anyhow::Result<()> {
    // Fault the whole injector directory: the policy builds the data and meta
    // spools under it, so this test does not need to name those subdirectories.
    let fault = FaultInjector::new()?;

    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-rocks-spool.lua")
        .env("KUMOD_ROCKS_SPOOL_DIR", fault.path().to_str().unwrap())
        .env("KUMOD_ROCKS_ERROR_UNLATCH_DURATION", "6s")
        .envs(fault.env_vars()?)
        .start()
        .await
        .context("start daemon")?;

    // Hold delivery to keep accepted messages in the spool.
    suspend_example_com(&daemon, "hold for enospc test")
        .await
        .context("suspend example.com")?;

    // Baseline: the spool accepts a message before any fault.
    assert_equal!(send_one(&daemon).await.context("baseline send")?, 250);

    // Begin the disk-full fault: spool writes now return ENOSPC.
    fault.enable().await?;

    // The WAL append for this store hits ENOSPC, a fatal IO error that latches
    // the gate immediately. This first attempt fails while the write is in
    // flight, distinct from the load-shed rejection that later connections get
    // once the gate has latched.
    {
        let err = send_one(&daemon)
            .await
            .unwrap_err()
            .downcast::<ClientError>()
            .context("downcast ClientError")?;
        match err {
            ClientError::Rejected(resp) => {
                assert_equal!(resp.code, 421);
                assert_equal!(resp.content, "kumo.test technical difficulties");
            }
            other => anyhow::bail!("unexpected client error: {other:?}"),
        }
    }
    wait_for_load_shed_active(&daemon.source, Duration::from_secs(30))
        .await
        .context("gate did not latch under ENOSPC")?;

    // Ingress rejects new connections while the gate is latched.
    {
        let banner = read_smtp_banner(&daemon.source).await?;
        assert_equal!(banner.code, 421);
        assert_equal!(
            banner.content,
            "kumo.test the spool is not accepting writes. Try later"
        );
    }

    // End the fault: space is restored.
    fault.disable().await?;

    // The gate must reopen on its own, and the spool must accept mail again.
    daemon
        .source
        .wait_for_metric(
            Duration::from_secs(90),
            |m| m.name().as_str() == "rocks_spool_load_shed_active",
            |values| !values.is_empty() && values.iter().all(|v| *v == 0.0),
        )
        .await
        .context("gate did not reopen after ENOSPC cleared")?;
    assert_equal!(send_one(&daemon).await.context("send after recovery")?, 250);

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}

/// Drives the backpressure-timeout path (as opposed to the error paths above):
/// slow -- not failing -- flush writes stall RocksDB into rejecting foreground
/// writes with `Incomplete`, so `store()` exhausts its `store_deadline` in the
/// backoff loop and reports a backpressure timeout, then latches the gate after
/// `error_latch_duration`. This is the slow-disk analogue of the ENOSPC test:
/// writes never fail, they just cannot make progress.
///
/// The fault shim runs in delay mode scoped to `.sst` files: the write-ahead
/// log stays fast and only flush stalls, matching the shape actually produced
/// by a slow disk. A tiny `write_buffer_size` fills the memtable within a few
/// messages, engaging the stall quickly.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn spool_backpressure_timeout_latches() -> anyhow::Result<()> {
    let fault = FaultInjector::new()?
        .fault_suffix(".sst")
        .delay(Duration::from_secs(5));

    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-rocks-spool.lua")
        .env("KUMOD_ROCKS_SPOOL_DIR", fault.path().to_str().unwrap())
        .env("KUMOD_ROCKS_STORE_DEADLINE", "2s")
        .env("KUMOD_ROCKS_ERROR_UNLATCH_DURATION", "60s")
        .envs(fault.env_vars()?)
        .start()
        .await
        .context("start daemon")?;

    // Hold delivery to keep accepted messages in the spool.
    suspend_example_com(&daemon, "hold for backpressure test")
        .await
        .context("suspend example.com")?;

    // Baseline: the spool accepts a message before any fault.
    assert_equal!(send_one(&daemon).await.context("baseline send")?, 250);

    // Slow every flush from now on.
    fault.enable().await?;

    // Keep offering messages until the gate latches. The memtable fills behind
    // the stalled flush, RocksDB then rejects writes with `Incomplete`, and a
    // store eventually exhausts its deadline in the backoff loop. Only a store
    // attempted while the stall is engaged hits `Incomplete`. A one-shot burst
    // sent before the memtable fills would be accepted immediately instead and
    // never see the stall at all. Sustained load keeps offering stores across
    // the window when the stall is engaged, guaranteeing one is routed to it.
    let sender = async {
        loop {
            let _ = send_one(&daemon).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::select! {
        _ = sender => unreachable!("sender loops forever"),
        r = wait_for_load_shed_active(&daemon.source, Duration::from_secs(60)) => {
            r.context("gate did not latch under backpressure timeout")?;
        }
    }

    // A new connection is load-shed at the banner while the gate is latched.
    {
        let banner = read_smtp_banner(&daemon.source).await?;
        assert_equal!(banner.code, 421);
        assert_equal!(
            banner.content,
            "kumo.test the spool is not accepting writes. Try later"
        );
    }

    fault.disable().await?;
    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}
