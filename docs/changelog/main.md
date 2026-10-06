# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

 * You may now monitor the status of your log consumers via kumod metrics. For
   [configure_local_logs](../reference/kumo/configure_local_logs/index.md) and
   [kumo.jsonl.new_writer](../reference/kumo.jsonl/new_writer.md), the status of
   jsonl log segments is reported based on the checkpoint files maintained in
   the corresponding log directories. The new
   [log_consumer_segments_behind](../reference/metrics/kumod/log_consumer_segments_behind.md),
   [log_consumer_bytes_behind](../reference/metrics/kumod/log_consumer_bytes_behind.md),
   and
   [log_consumer_lag_seconds](../reference/metrics/kumod/log_consumer_lag_seconds.md)
   metrics expose this information.

## Fixes

 * When started as root with `--user`, kumod now sets the real, effective and
   saved user and group ids to the target user, rather than lowering only the
   effective user id. Previously the real and saved ids remained `0`, which
   could potentially allow code running inside kumod to call `setresuid(0,0,0)`
   and regain full root privilege.  No such code exists in kumod itself, but
   it presented a potential avenue for an attacker, if they could contrive
   for kumod to execute arbitrary code through some other vulerability.
   to remote code execution vulnerabilities are known to exist.
   kumod now retains only `CAP_NET_BIND_SERVICE`, which it needs to bind
   privileged ports.

