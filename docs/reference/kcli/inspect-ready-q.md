---
tags:
  - ops
  - debugging
---
# kcli inspect-ready-q


Returns information about a ready queue: its egress identity, effective state (throttles, suspensions, ready and connection counts), and, optionally, the dispatcher tasks that are currently handling its connections plus the egress path configuration in effect


**Usage:** `kcli inspect-ready-q [OPTIONS] <QUEUE_NAME>`

## Arguments


* `<QUEUE_NAME>` — The name of the ready queue to inspect

## Options


* `--connections` — Show the per-connection dispatcher state (phase, time in phase, message counters, etc.). Off by default since a busy queue can have many dispatchers

* `--config` — Include the egress path configuration snapshot. Off by default because the config can be large

* `--json` — Output the response as pretty-printed JSON. Mutually exclusive with the other flags; the JSON output always carries the full payload



