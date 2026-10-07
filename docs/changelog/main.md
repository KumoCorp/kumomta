# Unreleased Changes in The Mainline

## Breaking Changes

* Site-name generation now preserves complete hostname branches instead of
  combining labels independently. Distinct MX sets that previously collided
  now have separate site names and ready queues. For example, two real-world
  Zoho MX sets:

  ```text
  Set A: mx.zoho.com, mx2.zoho.com, mx3.zoho.eu
  Set B: mx.zoho.com, mx2.zoho.eu,  mx3.zoho.com

  Before (both): (mx|mx2|mx3).zoho.(com|eu)
  After A:       ((mx|mx2).zoho.com|mx3.zoho.eu)
  After B:       ((mx|mx3).zoho.com|mx2.zoho.eu)
  ```

  This affects the site names that you may observe in metrics and kcli command
  output.  It is recommended that you review whether you have hardcoded any
  assumptions about the site name in your monitoring/orchestration integration
  prior to upgrading.  We do not anticipate this affecting anyone in practice.
  See [Site Names](../reference/queues.md#site-names) for details.

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

 * DKIM relaxed body canonicalization now reduces an empty body, or a body
   consisting only of empty or whitespace-only lines, to zero octets as required
   by RFC 6376 section 3.4.4. Previously it retained a CRLF, producing a body
   hash that disagreed with verifiers such as Gmail and causing otherwise-valid
   signatures to fail. Thanks to @bjarn! #575

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

 * [kumo.generate_rfc3464_message](../reference/kumo/generate_rfc3464_message.md)
   no longer fails to produce a bounce when the original message has 8-bit
   headers or body. The returned copy of the original is encoded to keep the
   report 7-bit clean, and downgrades to just the headers, or is omitted, when
   its content cannot be represented that way.

 * The [mail_auth](../reference/policy-extras.mail_auth/index.md) iprev check now
   follows at most 10 of the connecting IP's PTR names with forward A/AAAA
   lookups, as required by RFC 8601 section 3. Previously it followed every
   name, letting the owner of the reverse zone drive an unbounded number of DNS
   lookups per connection. #623

 * The SMTP connection plan built for a delivery attempt is now bounded by two
   new [make_egress_path](../reference/kumo/make_egress_path/index.md) options,
   [max_mx_addresses_per_host](../reference/kumo/make_egress_path/max_mx_addresses_per_host.md)
   (default `10`) and
   [max_mx_plan_size](../reference/kumo/make_egress_path/max_mx_plan_size.md)
   (default `50`). Previously MX resolution retained every address of every MX
   host with no cap, which meant that the connection plan could be arbitrarily
   large. #622

 * Fixed a DANE downgrade that could occur when an A or AAAA lookup returned a
   bogus (DNSSEC validation failure) result. Such a result now defers delivery
   rather than being treated as ordinary unsigned addresses. #612
