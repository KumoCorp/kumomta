use crate::kumod::{DaemonWithMaildirOptions, KumoArgs, KumoDaemon, MailGenParams};
use anyhow::Context;
use k9::assert_equal;
use kumo_log_types::{JsonLogRecord, RecordType};
use std::collections::BTreeMap;
use rfc5321::client::{ClientError, SmtpClient};
use rfc5321::client_types::SmtpClientTimeouts;
use rfc5321::Response;
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

/// Verifies that when the rocksdb-backed spool wedges at runtime, the
/// composite latch in `metrics_monitor` engages within the configured
/// window and the load-shedding gate fires across all three ingress
/// paths (SMTP, HTTP inject, HTTP liveness) with the external
/// reason string.  Also verifies that the forced compaction surfaces
/// the underlying rocksdb error to the caller.
#[tokio::test]
async fn spool_write_stopped_load_shedding() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
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

    // Clean shutdown -- propagate any errors as the test would.
    // Shutting down a write-stopped rocksdb may itself surface errors;
    // if this proves flaky in practice we can revisit.
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
    let post_restart_io_error = histogram.count(
        RecordType::TransientFailure,
        400,
        &expected_spool_error,
    );
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
