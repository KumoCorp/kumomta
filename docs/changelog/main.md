# Unreleased Changes in The Mainline

## Breaking Changes

 * SMTP AUTH PLAIN is no longer sent over a TLS session whose peer
   certificate was not validated (for example an
   `OpportunisticInsecure`/`RequiredInsecure` session, or a DANE host with
   unusable TLSA records). Set the new
   [allow_smtp_auth_plain_without_valid_certificate](../reference/kumo/make_egress_path/allow_smtp_auth_plain_without_valid_certificate.md)
   egress path option to `true` to restore the previous behavior.

 * Rocksdb-backed spool `store()` and `remove()` calls now time out
   after 30 seconds of backpressure rather than blocking
   indefinitely. Tunable via the new
   [store_deadline](../reference/kumo/define_spool/rocks_params.md#store_deadline)
   rocks_params field.

 * The `resolve-shaping-domain` script's default output has changed.
   Pass `--json-config` to restore the previous byte-for-byte
   pretty-JSON output of the path config. Run
   `resolve-shaping-domain --help` for the full flag list.

 * DNS resolver configuration is now defined by a kumomta-owned schema
   rather than forwarding hickory option names. Existing valid configs
   continue to parse, but unknown fields in `options` are now a
   configure-time error rather than being silently ignored, and the
   simple `'IP:PORT'` form of a name server entry now configures both
   UDP and TCP for that server (previously UDP only). See
   [configure_resolver](../reference/kumo.dns/configure_resolver.md) and
   the [Resolver Options reference](../reference/kumo.dns/resolver_options/index.md)
   for the supported fields.

## Other Changes and Enhancements

 * [enable_dane](../reference/kumo/make_egress_path/enable_dane.md) can now be
   used with the Hickory resolver (with DNSSEC validation enabled); it no
   longer requires the unbound resolver.

 * New [treat_mx_list_as_secure](../reference/kumo/make_queue_config/protocol.md#treat_mx_list_as_secure)
   SMTP protocol option. When set, the hosts in an `mx_list` are treated as a
   trusted (DNSSEC-secure) MX selection, allowing DANE to apply to a statically
   configured relay that does not go through `MX` resolution.

 * DANE now engages for an MX host that is a securely published `CNAME` whose
   target lands in an unsigned zone (RFC 7672 section 2.2.2): the `TLSA` records
   are queried at the original MX name and authenticate the peer even though the
   address records resolve insecurely. See
   [CNAME MX hosts](../reference/kumo/make_egress_path/enable_dane.md#cname-mx-hosts).

 * The [trust_anchor_file](../reference/kumo.dns/resolver_options/trust_anchor_file.md)
   resolver option now also accepts a `{ managed = "<path>" }` form, naming an
   RFC 5011 auto-maintained DNSSEC trust anchor file that stays current across
   root KSK rollovers without operator intervention. Supported by the unbound
   backend only.

 * Upgraded the embedded hickory-resolver 0.26 and libunbound 1.25.1. New
   [kumo.dns.load_resolv_conf](../reference/kumo.dns/load_resolv_conf.md)
   reads a resolv.conf-format file into a mutable resolver config table,
   so you can start from the system upstream list and layer your own
   `options` on top before calling
   [configure_resolver](../reference/kumo.dns/configure_resolver.md).

 * Egress sources can now be configured to auto-suspend when their
   local bind address appears unplumbed
   ([suspend_when_unplumbed](../reference/kumo/make_egress_source/suspend_when_unplumbed.md))
   or when their configured proxy server appears unreachable
   ([suspend_when_proxy_unhealthy](../reference/kumo/make_egress_source/suspend_when_proxy_unhealthy.md)).
   A suspended source is skipped during pool selection until the
   configured duration elapses. The trigger uses the same
   `Immediate` / `Threshold("N/period")` shape as TSA shaping rules.

 * [ha_proxy_server](../reference/kumo/make_egress_source/ha_proxy_server.md)
   and
   [socks5_proxy_server](../reference/kumo/make_egress_source/socks5_proxy_server.md)
   now accept a DNS host name in addition to an IP literal. The name is
   resolved at connection time and each returned address is tried in
   turn, sharing the `connect_timeout` budget.

 * KumoMTA now proactively detects when the rocksdb-backed spool has
   reached a state that requires operator intervention (a missing or
   corrupt SST surfaced through a foreground read/write, or sustained
   background-error accumulation from compactions or flushes) and
   transitions into a load-shedding state. While the spool is
   unhealthy, the SMTP banner returns 421, HTTP injection and
   `/api/check-liveness/v1` return 503, and delivery is paused.
   Pausing delivery limits the window in which a successful SMTP
   transaction could be followed by a failed spool `remove()`, which
   would otherwise cause that message to be redelivered. The
   diagnostic log records each transition that drives this: when
   the rocksdb `background-errors` counter grows, when a foreground
   read or write returns a fatal `IOError` or `Corruption`, when the
   load-shedding gate latches, and (where applicable) when the gate
   later auto-clears after sustained recovery. Each record names
   the spool path and points at the rocksdb LOG file in that
   directory for the underlying cause. The delivery pause itself
   can be toggled with the new
   [kumo.suspend_delivery_when_spool_unhealthy](../reference/kumo/suspend_delivery_when_spool_unhealthy.md)
   policy function (default: enabled). Several new metrics expose
   the underlying state to monitoring:
   [rocks_spool_load_shed_active](../reference/metrics/kumod/rocks_spool_load_shed_active.md),
   [rocks_spool_background_errors](../reference/metrics/kumod/rocks_spool_background_errors.md),
   [rocks_spool_write_stopped](../reference/metrics/kumod/rocks_spool_write_stopped.md),
   [rocks_spool_compaction_pending](../reference/metrics/kumod/rocks_spool_compaction_pending.md),
   [rocks_spool_num_running_compactions](../reference/metrics/kumod/rocks_spool_num_running_compactions.md),
   [rocks_spool_estimate_pending_compaction_bytes](../reference/metrics/kumod/rocks_spool_estimate_pending_compaction_bytes.md),
   and
   [rocks_spool_actual_delayed_write_rate](../reference/metrics/kumod/rocks_spool_actual_delayed_write_rate.md).

 * New [kcli spool-compact](../reference/kcli/spool-compact.md) command
   (and matching `/api/admin/spool-compact/v1` endpoint) forces a flush
   and full-keyspace compaction on a named rocksdb spool. Primarily a
   diagnostic and operational helper; surfaces underlying storage
   errors to the caller.

 * Ready queues now run a per-dispatcher progress watchdog that aborts
   dispatcher tasks that have stopped making forward progress, catching
   wedges that escape the normal SMTP timeouts. The threshold is
   configurable via
   [dispatcher_progress_watchdog_timeout](../reference/kumo/make_egress_path/dispatcher_progress_watchdog_timeout.md)
   and aborts are surfaced via the
   [dispatcher_watchdog_aborted_total](../reference/metrics/kumod/dispatcher_watchdog_aborted_total.md)
   metric.  #539

 * Added the
   [kcli inspect-ready-q](../reference/kcli/inspect-ready-q.md)
   command and corresponding
   [admin/inspect-ready-q/v1](../reference/http/kumod/api_admin_inspect_ready_q_v1_get.md)
   HTTP endpoint, which return a snapshot of a ready queue's state,
   effective configuration, the dispatcher tasks currently handling
   its connections, and the steady-state throughput ceilings implied
   by the egress path config.

 * Added the
   [kcli abort-ready-q-conn](../reference/kcli/abort-ready-q-conn.md)
   command and corresponding
   [admin/abort-ready-q-conn/v1](../reference/http/kumod/api_admin_abort_ready_q_conn_v1_post.md)
   HTTP endpoint, which abort a specific dispatcher task by
   `session_id`, as shown by the `inspect-ready-q` output.

 * Added the
   [kcli resolve-egress-path](../reference/kcli/resolve-egress-path.md)
   command and corresponding
   [admin/resolve-egress-path/v1](../reference/http/kumod/api_admin_resolve_egress_path_v1_get.md)
   HTTP endpoint, which report the effective egress path config,
   scheduled-queue config, MX resolution, ready-queue name and the
   throughput ceilings derived from both configs for a destination
   domain and egress source. Equivalent to running
   `resolve-shaping-domain` against the live runtime instead of a
   static policy file.

 * Added new lua functions:
   [kumo.compute_egress_path_config_constraints](../reference/kumo/compute_egress_path_config_constraints.md),
   [kumo.compute_egress_path_config_constraints](../reference/kumo/compute_egress_path_config_constraints.md),
   [kumo.compute_queue_config_constraints](../reference/kumo/compute_queue_config_constraints.md),
   [kumo.format_egress_path_config_constraints](../reference/kumo/format_egress_path_config_constraints.md),
   [kumo.format_egress_path_config_toml](../reference/kumo/format_egress_path_config_toml.md),
   [kumo.serde.toml_encode_pretty_compact](../reference/kumo.serde/toml_encode_pretty_compact.md).

 * `resolve-shaping-domain` now shows the resolved configuration in
   a pretty toml output, including both the scheduled-queue config
   and the egress path config, and shows the same throughput-ceiling
   diagnostic as `kcli inspect-ready-q` with constraints from both
   configs folded in. A new `--json-queue-config` flag emits the
   queue config as pretty JSON.

 * `kcli inspect-ready-q` gains an opt-in `--sched-q` flag, and the
   corresponding HTTP endpoint accepts an `include_scheduled_queues`
   query parameter, that returns the list of scheduled queue names
   currently feeding the ready queue. Useful for tracing fan-in
   when many domains or tenants converge on a single destination.

 * Community shaping.toml file had updates to domains qq.com, 163.com, and yahoo.co.jp and providers gmail, yahoo, outlook, apple, orange, and mimecast. New provider definitions were added for barracuda (barracudanetworks.com), netvigator (netvigator.com), and kpn (kpnmail.nl). Note that web.de moved from a domain to being a provider named gmx.net_web.de (which also matches .gmx.net), and qq.com and 163.com were moved from domain-level to provider-level automations. Thanks to @Solmea! #531

 * When running under a cgroup memory limit, `memory_usage` is now the
   cgroup *working set* (`memory.current - inactive_file`, floored by
   anonymous memory) rather than the raw `memory.current`. This excludes
   cold, kernel-reclaimable file-backed page cache, which the kernel drops
   before it would OOM-kill the container, and matches what kubelet/cAdvisor
   report as `container_memory_working_set_bytes`. It removes premature load
   shedding caused by phantom cache pressure for workloads that write a lot of
   logs or spool to disk. **This lowers the observed `memory_usage` for any
   deployment under a cgroup limit; alerts keyed on `memory_usage` may need
   re-baselining.** See
   [Memory Management](../reference/memory.md#working-set-under-a-cgroup).
   Thanks to @dschaaff! #549

 * We now perform extended permission related probing on the spool and maildir
   directory locations to catch uncommon permission misconfigurations on
   startup, that would otherwise lead to rocksdb corrupting itself on
   the restart *after* the permissions were broken.

## Fixes

 * An SMTP command line containing bytes that are not valid UTF-8 is now
   rejected with a `501` syntax error and the session continues, rather
   than aborting the connection with a `421 technical difficulties`
   response. #550

 * [DANE](../reference/kumo/make_egress_path/enable_dane.md) (RFC 7672)
   support was too permissive and is now downgrade resistant. #543

 * `Message::save_to` was silently discarding errors returned from the
   data and meta spool `store()` operations: the per-spool dirty flags
   were cleared regardless of success, so a message that failed to
   persist was still treated by the SMTP ingress path as accepted.
   Errors now propagate so the ingress path can reject (and the client
   retries) instead of producing a silent loss.
