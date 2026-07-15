---
description: "Why KumoMTA restart or shutdown takes several minutes — connection timeouts, TimeoutStopSec force-kills, and tuning system_shutdown_timeout."
---

# Why Does Restart or Shutdown Take Several Minutes (or Hang)?

When KumoMTA shuts down it tries to finish in-flight delivery attempts and flush state to spool before exiting. Open SMTP connections are not killed instantly — each in-flight attempt is allowed to run out its per-phase connection timeouts ([`connect_timeout`](../reference/kumo/make_egress_path/connect_timeout.md), [`mail_from_timeout`](../reference/kumo/make_egress_path/mail_from_timeout.md), [`data_timeout`](../reference/kumo/make_egress_path/data_timeout.md), and so on) before it is aborted. A session stalled against a slow or unresponsive remote is the single biggest reason a clean shutdown takes time. Idle connections held open for reuse ([`idle_timeout`](../reference/kumo/make_egress_path/idle_timeout.md), default `60s`) also have to close during shutdown, though those drain quickly. Anything unfinished is recovered from the spool on the next start, and no messages are lost.

A shutdown that takes around 5 minutes usually means the systemd stop timeout (`TimeoutStopSec`) was reached and the process was force-killed. On a correctly configured server you should rarely hit that limit.

## What to do

Prefer `systemctl stop` followed by `systemctl start` over `systemctl restart`, especially after rapid successive restarts.

If shutdown regularly hangs, trace the journal in debug mode to find what is still busy just before the SIGKILL. It is typically a misconfiguration or a slow, blocking call in a delivery handler:

```console
$ kcli set-log-filter 'kumod=debug'
$ journalctl -f -u kumomta.service
```

Then tune `system_shutdown_timeout`, which bounds the whole drain. Its default is the sum of the per-message SMTP timeouts (`mail_from_timeout` + `rcpt_to_timeout` + `data_timeout` + `data_dot_timeout`), so if you have raised any of those connection timeouts you have also raised how long shutdown can take. Set it so the server has enough time to drain in-flight messages cleanly, but not so much that a few stuck connections force every restart to hang.

!!! note
    A long **startup** (spool enumeration) is a different problem — that one is usually disk-bound. See the troubleshooting guide if startup, rather than shutdown, is slow.

## See also

* [make_egress_path / system_shutdown_timeout](../reference/kumo/make_egress_path/system_shutdown_timeout.md)
* [Troubleshooting KumoMTA](../userguide/operation/troubleshooting.md)
