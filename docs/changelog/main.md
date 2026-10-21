# Unreleased Changes in The Mainline

## Breaking Changes

* The `LocalDisk` spool kind is deprecated and slated for removal in a future
  release. It has not yet been removed, but defining a spool with it now logs
  an error to advise you of the future removal. Use the `RocksDB` kind instead;
  it is faster, more durable, uses less space (and thus IOPS), and is the
  recommended production configuration.  In addition, we've made the
  [kind](../reference/kumo/define_spool/kind.md) field of
  [kumo.define_spool](../reference/kumo/define_spool/index.md) a required field
  so that new users don't accidentally deploy with `LocalDisk` between now and
  the removal of the `LocalDisk` support.

* [kumo.dns.configure_unbound_resolver](../reference/kumo.dns/configure_unbound_resolver.md)
  is deprecated and slated for removal in a future release. It is not yet
  removed, but calling it now logs an error to advise you of the future removal.
  Use [kumo.dns.configure_resolver](../reference/kumo.dns/configure_resolver.md)
  instead.  The only reason to consider using the `configure_unbound_resolver`
  was if you required DANE support, but the hickory resolver has been able to
  satisfy that requirement since version `2026.09.22-a276d4a8` and works more
  reliably in KumoMTA.

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

* The HTTP injection API now bounds the number of concurrently processed
  requests (default `32768`), configurable via
  [kumo.set_httpinject_concurrency_limit](../reference/kumo/set_httpinject_concurrency_limit.md).
  Requests above the limit are rejected with a `503` response rather than
  queued. The new
  [httpinject_requests_in_flight](../reference/metrics/kumod/httpinject_requests_in_flight.md)
  gauge and
  [httpinject_requests_rejected_total](../reference/metrics/kumod/httpinject_requests_rejected_total.md)
  counter expose the current load and rejections. #631

## Other Changes and Enhancements

 * Implicit TLS (SMTPS) is now supported on both the inbound and outbound SMTP
   paths.
   [kumo.start_esmtp_listener](../reference/kumo/start_esmtp_listener/index.md)
   accepts a [tls_mode](../reference/kumo/start_esmtp_listener/tls_mode.md)
   parameter, and
   [kumo.make_egress_path](../reference/kumo/make_egress_path/index.md) accepts
   a [tls_mode](../reference/kumo/make_egress_path/tls_mode.md) parameter, each
   selecting `StartTls` (the default) or `ImplicitTls`. #199 #200

 * New [kumo.string.psl_suffix_is_known](../reference/string/psl_suffix_is_known.md)
   and [kumo.string.psl_domain_is_known](../reference/string/psl_domain_is_known.md)
   report whether the resolved public suffix is explicitly listed in the Public
   Suffix List. Thanks to @umutondersu! #592

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

 * The `log_hooks` helper
   [new_disposition_hook](../reference/kumo/configure_log_disposition_hook.md#using-the-log_hooks-helper)
   now accepts a `log_parameters` table and forwards it to
   [kumo.configure_log_disposition_hook](../reference/kumo/configure_log_disposition_hook.md),
   letting you set options such as `per_record` through the helper. #499 #500 #519 #520

 * [kumo.memoize](../reference/kumo/memoize.md) caches now run the population
   function that fills a cache miss on a separate task by default, rather than
   on the calling task. A caller that is cancelled (an HTTP request handler
   whose client disconnected) no longer cancels the cache population function.
   The new `detached` memoize option can be used to override this if needed.  #602

 * The [HTTP injection API](../reference/http/kumod/api_inject_v1_post.md)
   now caches compiled templates across requests. Batch injection commonly
   sends the same `text_body`, `html_body`, `amp_html_body` or `content`
   template in many requests, each of which previously compiled it again. It
   is now compiled once and reused by later requests with the same template
   text and `template_dialect`. Content that contains no template syntax is
   not cached. The cache is enabled by default and holds up to 1,000 templates
   for up to 10 minutes. Use
   [kumo.set_lruttl_cache_capacity](../reference/kumo/set_lruttl_cache_capacity.md)
   with the cache name `http_inject_template` to change its capacity, or
   [kumo.set_httpinject_template_cache_enabled](../reference/kumo/set_httpinject_template_cache_enabled.md)
   to disable it.

 * Improved [HTTP injection API](../reference/http/kumod/api_inject_v1_post.md)
   performance for `content` fields and headers that are effectively static.
   Previously these would round-trip through the template engine, but now we
   recognize that we can precompile them once for the whole request and skip
   the engine overhead.  In a microbenchmark of expanding a marker-free value,
   this is 50-100x faster than routing it through the engine, depending on the
   template dialect. The benefit scales with the number of static fields and
   recipients in a batch injection. Thanks to @shivamgoel1009! #620

 * Further improved [HTTP injection API](../reference/http/kumod/api_inject_v1_post.md)
   performance for the `content` fields and headers that do vary per recipient.
   Previously the substitution context was re-serialized into the template
   engine's native form for every field expanded, but now we convert it once
   per recipient and render every field against that prepared context. In a
   microbenchmark this expands fields roughly 2-3x faster with the Jinja
   dialect and 3-5x faster with Handlebars.

## Fixes

 * `AUTH PLAIN` credentials are now redacted from both inbound rejection logs
   and outbound delivery failure records, replaced with `[REDACTED]` in all log
   and diagnostic output. Thanks to @terrorobe! #636

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

 * Fixed the handling of a failed MX lookup on the deprecated unbound resolver
   backend.  A DNS *failure* response (SERVFAIL, REFUSED, or any other
   non-NXDOMAIN failure RCODE) is now classified as a temporary error instead,
   and it no longer qualifies for the implicit `MX->A` fallback and is treated
   as an ordinary failed lookup, as RFC 5321 section 5.1 requires.  The default
   hickory backend already reports these failures as errors and its behavior is
   unchanged. #629

 * A DNSSEC-bogus MX answer (one that failed DNSSEC validation) is no longer
   used to route mail. The MX lookup now treats a bogus result as a temporary
   error and defers, rather than using forged or tampered MX hosts as an
   ordinary unsigned MX set, as RFC 7672 section 2.1.1 requires. This is the MX
   counterpart of the A/AAAA bogus fix above (#612). It applies both to the
   deprecated unbound backend and to a hickory backend that has been
   explicitly configured to perform DNSSEC validation (`validate = true`)
   against an upstream that does not validate (a validating upstream returns a
   SERVFAIL for bogus data, which hickory handles as an error). #629

 * A dual-stack address lookup where one family (A or AAAA) failed while the
   other succeeded no longer holds the partial result for the surviving
   family's full TTL, which could be up to a day. We now use the negative-cache
   TTL (60 seconds) to ensure that we will re-attempt the failed address family
   in a more reasonable time frame.  NXDOMAIN and NODATA answers are not
   failures and keep their normal TTL. #630

 * An explicit [mx_list](../reference/kumo/make_queue_config/protocol.md) with
   multiple entries would error out of the SMTP dispatcher setup if one of the
   entries failed to resolve, instead of continuing with a partial list of
   hosts. This could prevent delivery to that destination for as long as the
   entry failed to resolve, without a clear log of the reason. We now continue
   with whatever resolved in the partial success case, and include the failed
   DNS lookup results in the TransientFailure log record that we produce if they
   all fail. #628

 * Reduced lua context creation overhead in scheduled queue config refreshes,
   egress source and pool loading and `kumo.secrets` loading
   by reusing the cached context. Thanks to @edgarsendernet! #632

 * The
   [message_data_resident_count](../reference/metrics/kumod/message_data_resident_count.md)
   metric could drift away from the true number of messages holding their body
   in memory: constructing a message with an empty body leaked the count
   upward, while messages received via node-to-node transfer were never counted
   and drove it downward (potentially negative).  The count is now bound to the
   lifetime of the resident body, which keeps it accurate in both directions.
   Thanks to @kayozaki! #588

 * KumoMTA no longer continues in cleartext after a failed STARTTLS handshake,
   on either the outbound or the inbound SMTP path. Thanks to @terrorobe for
   reporting and analyzing this in #615!

   On the outbound path, a failed handshake with
   [enable_tls](../reference/kumo/make_egress_path/enable_tls.md) set to
   `Opportunistic` or `OpportunisticInsecure` now fails the connection attempt
   rather than continuing the session in cleartext. Delivery behavior is
   unchanged; only the logged text for such a failure changes, from:

   ```text
   mx.example.com/192.0.2.1:25: EHLO after OpportunisticInsecure STARTTLS handshake status: failed: <tls error>
   ```

   to:

   ```text
   TLS handshake with ResolvedAddress { name: "mx.example.com", addr: 192.0.2.1, is_secure: false }:25 failed: <tls error>
   ```

   On the inbound SMTP listener, a failed STARTTLS handshake now drops the
   connection instead of continuing to read commands in cleartext.

