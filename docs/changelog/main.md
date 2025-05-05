# Unreleased Changes in The Mainline

## Breaking Changes
* The embedded hickory DNS resolver was updated to version `0.25`.
  If you are using
  [kumo.dns.configure_resolver](../reference/kumo.dns/configure_resolver.md) be
  aware that hickory has changed its configuration schema and that you may need
  to update your configuration to match; be sure to test this before trying to
  deploy to production.
* There is now an artificial limit of `128` concurrent MX lookups that are
  permitted to be in-flight at any given time. In prior versions there was
  no limit. You can adjust this via the new
  [kumo.dns.set_mx_concurrency_limit](../reference/kumo.dns/set_mx_concurrency_limit.md)
  function.
* *dev* docker images are now published as `ghcr.io/kumocorp/kumomta:main`
  instead of `ghcr.io/kumocorp/kumomta-dev:latest`. The most recent version
  of the older image will be retained for a while, but you should
  update to reference the new tag.

## Other Changes and Enhancements

* SMTP Server: TLS parameters will now be cached for up to 5 minutes at
  a time, making it easier for a server to pick up an updated certificate
  file. In prior versions, the TLS parameters would be held for the lifetime
  of the process, requiring a restart to pick up a changed certificate/key
  pair.
* SMTP Server: new [via](../reference/kumo/start_esmtp_listener/via.md),
  [peer](../reference/kumo/start_esmtp_listener/peer.md), and
  [meta](../reference/kumo/start_esmtp_listener/meta.md) options for
  SMTP listeners enable metadata (and other existing listener options) to
  be conditionally set based on the source and local address of the
  incoming SMTP session.
* SMTP Server: new
  [smtp_server_connection_accepted](../reference/events/smtp_server_connection_accepted.md)
  event allows custom processing prior to returning the banner to the client.
* SMTP Server: new
  [smtp_server_get_dynamic_parameters](../reference/events/smtp_server_get_dynamic_parameters.md)
  event allows dynamically amending listener configuration to support IP-based
  virtual service.
* Updated the hickory DNS resolver to `0.25`. While no kumomta-user-visible
  changes are anticipated as a result of this upgrade, it is a fairly
  significant release of the DNS resolver so please report unexpected
  changes in behavior around DNS.
* Added example and recommended default shaping configuration for the
  TSA daemon to the default `shaping.toml` file.
* If you are running on a system where
  [kumo.available_parallelism](../reference/kumo/available_parallelism.md)
  returns an inaccurate value (such as an overcommitted VM), then you may
  now export `KUMO_AVAILABLE_PARALLELISM` into the launching environment to
  override the value to something more appropriate, which helps to scale
  the various thread pools more appropriately.
* New systemd environment override files for both kumod and tsa-daemon.  These
  are useful for setting the `KUMO_AVAILABLE_PARALLELISM` variable mentioned in
  the item above. See [the
  commit](https://github.com/KumoCorp/kumomta/commit/f8bbacba541375e0be2d2ac355f4c109826c0700)
  for details.
* Ready Queue Maintenance is now carried out in a new `readyq_qmaint` thread
  pool. In previous versions, it was handled by the `qmaint` thread pool.  If
  you were tuning via
  [kumo.set_qmaint_threads](../reference/kumo/set_qmaint_threads.md) in a prior
  version, you may need to review and adjust both that and the corresponding
  [kumo.set_ready_qmaint_threads](../reference/kumo/set_ready_qmaint_threads.md)
  tuning.
* New
  [system_shutdown_timeout](../reference/kumo/make_egress_path/system_shutdown_timeout.md)
  option to specify how long we should wait for an in-flight delivery attempt
  to wrap up before terminating it once we have received a request to shutdown
  kumod.
* `kcli top`: you may now scroll through the metrics using the arrow keys
  (to move one metric at a time), page up/down (10 at a time) and home/end
  (to move to the top/bottom). Pressing `f` edits a fuzzy matching filter.
  Pressing Tab moves through tabs and allows viewing heatmap vizualizations.
  #372
* Disabled DANE in the default `shaping.toml` for the `office365-dane` provider.
  We cannot default DANE to on without a guarantee that the DNS/resolver
  situation is correctly deployed with DNSSEC and without also knowing that
  we're configure to use openssl.
* MX lookups now participate in lruttl cache thundering herd protection.
  In prior versions, we could potentially issue multiple concurrent requests
  for the same name.
* Singleton wheel now spawns batches of messages when promoting them to
  the ready queue, helping to ensure more even time keeping when there are
  domains with slow DNS.
* Add caching and, more importantly, negative caching in the queue insertion
  code paths when checking for admin bounce entries. This improves performance
  on systems with a large number of admin bounce entries.
* Improved performance of TSA state storage, which in turn improves
  latency in tsa-daemon response times when many automation rules are triggering.
* New
  [maintainer_wakeup_strategy](../reference/kumo/make_egress_path/maintainer_wakeup_strategy.md)
  and
  [dispatcher_wakeup_strategy](../reference/kumo/make_egress_path/dispatcher_wakeup_strategy.md)
  options for fine tuning overall system performance.
* [log_arf](../reference/kumo/make_listener_domain/log_arf.md) and
  [log_oob](../reference/kumo/make_listener_domain/log_oob.md) now support
  `"LogThenDrop"` to make it easier to express a policy that wants to log
  incoming reports but not relay them. Additional descriptive dispositions for
  the prior behavior are now also supported, in addition to the legacy boolean
  values.
* The rfc5965 (ARF) parser is now more forgiving if the incoming message
  has an invalid `Arrival-Date` or `Received-Date` header, such as that
  included in one of the examples in rfc5965 itself.

## Fixes

* Specifying `validation_options` for the shaping helper without explicitly
  setting the new `http_timeout` could lead to a `missing field` error when
  running `kumod --validate`.
* tsa-daemon now increases its soft `NOFILE` limit to match its hard limit
  on startup (just as we do in kumod), which helps to avoid issues with
  running out of file descriptors on systems with very large core counts.
* Shutdown could take longer than the 300s permitted by kumomta.service
  when lua delivery handlers are experiencing delays, leading to systemd
  issuing a SIGKILL.
* Loading an ed25519 private key via `kumo.dkim.ed25519_signer` would always
  fail. #368
* [msg:get_data](../reference/message/get_data.md),
  [msg:get_meta](../reference/message/get_meta.md) and
  [msg:set_meta](../reference/message/set_meta.md) now internally will ensure
  that the data or meta portion of the message is loaded from spool.
* Rebuilding a MIME message (such as via `msg:check_fix_conformance`) that had
  binary attachments would incorrectly re-interpret the bytes as windows-1252
  encoded characters, damaging the attachment.
* Using redis-based throttles without redis-cell and with long periods (eg: `500/d`)
  could result in throttles being exceeded.
