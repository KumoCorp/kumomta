# Unreleased Changes in The Mainline

## Breaking Changes
* The embedded hickory DNS resolver was updated to version `0.25`.
  If you are using
  [kumo.dns.configure_resolver](../reference/kumo.dns/configure_resolver.md) be
  aware that hickory has changed its configuration schema and that you may need
  to update your configuration to match; be sure to test this before trying to
  deploy to production.

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
