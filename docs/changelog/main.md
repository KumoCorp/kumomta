# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

 * The SOCKS5 proxy listener now accepts a
   [max_connections](../reference/proxy/start_proxy_listener/max_connections.md)
   parameter (default `32768`) that bounds the number of concurrent client
   connections. Connections above the limit are closed immediately and counted
   by the new
   [proxy_connections_denied_total](../reference/metrics/proxy-server/proxy_connections_denied_total.md)
   metric.

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

 * The systemd unit now launches kumod directly as the `kumod` user, with
   `CAP_NET_BIND_SERVICE` granted ambiently, matching the configuration that
   kumod would establish for itself when spawned as root, but doing so without
   ever having full root privilege.  If you deploy your own unit file, add
   `User=kumod`, `Group=kumod`, `AmbientCapabilities=CAP_NET_BIND_SERVICE` and
   `CapabilityBoundingSet=CAP_NET_BIND_SERVICE` to adopt the same model.

## Fixes

 * SMTP and SOCKS5 proxy listeners no longer stop serving when `accept` returns
   an error. Previously a transient error such as file-descriptor exhaustion
   (`EMFILE`/`ENFILE`) propagated out of the accept loop and permanently halted
   the listener while the process kept running. Because the process itself
   stayed up, a supervisor such as systemd's `Restart=always` saw a healthy
   process and would not restart it, leaving only a manual service restart to
   recover the listener. The listeners now log and continue, pausing briefly on
   resource-exhaustion errors to avoid spinning.

 * When started as root with `--user`, kumod now sets the real, effective and
   saved user and group ids to the target user, rather than lowering only the
   effective user id. Previously the real and saved ids remained `0`, which
   could potentially allow code running inside kumod to call `setresuid(0,0,0)`
   and regain full root privilege.  No such code exists in kumod itself, but
   it presented a potential avenue for an attacker, if they could contrive
   for kumod to execute arbitrary code through some other vulerability.
   No remote code execution vulnerabilities are known to exist.
   kumod now retains only `CAP_NET_BIND_SERVICE`, which it needs to bind
   privileged ports.

