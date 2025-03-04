# Unreleased Changes in The Mainline

## Breaking Changes
* New
  [data_processing_timeout](../reference/kumo/start_esmtp_listener/data_processing_timeout.md)
  sets a default time limit of 5 minutes for processing the DATA phase during
  SMTP reception.  If this limit is too short for your workflow, you will
  need to configure a larger value in your `start_esmtp_listener` setup.
* Removed deprecated `should_enqueue_log_record` and `get_queue_config` fields
  from the `shaper` object returned from `shaping:setup_with_automation`.
  These have had no effect for the past several stable releases, having been
  made automatic. If you are referencing them in your policy, you can simply
  remove the associated logic.
* MX resolution now has a default timeout of 5 seconds. Read
  [kumo.dns.set_mx_timeout](../reference/kumo.dns/set_mx_timeout.md) for more
  information.
* The HTTP client returned from `kumo.http.build_client` will now look for the
  system CA-certificate bundle when making connections. **If no CA-certificate
  bundle is present**, it will have no available trust store and will **not be
  able to successfully establish TLS sessions**. Previously, we used a bundled
  hard-coded, non-extensible, copy of the Mozilla CA certificate store. You
  must therefore ensure that you install the `ca-certificates` package for your
  system, or otherwise contrive to populate the system certificate store. Note
  that this change is consistent with a similar change to the SMTP client in
  the `2024.11.08-d383b033` release.

## Other Changes and Enhancements

* DKIM signer TTLs can be now be expressed using duration strings like `"5
  mins"`. Previously you could only use the integer number of seconds.
* debian packages will now unmask kumod and tsa-daemon services as part
  of post installation.  Thanks to @cai-n! #331
* [memoize](../reference/kumo/memoize.md) now has an optional
  `invalidate_with_epoch` parameter that allows you to opt a specific cache
  into epoch-based invalidation.
* DKIM signer has a separate supplemental cache for the parsed key data,
  which helps to reduce latency for deployments where the same key data
  is shared between multiple signing domains.
* New [msg:shrink()](../reference/message/shrink.md) and
  [msg:shrink_data()](../reference/message/shrink_data.md) methods.
* Added various python compatibility functions to the minijinja template engine.
  See [the pycompat
  docs](https://docs.rs/minijinja-contrib/latest/minijinja_contrib/pycompat/fn.unknown_method_callback.html)
  for a list of the additional functions.
* New [kumo.string.eval_template](../reference/string/eval_template.md)
  function for expanding minijinja template strings.
* New [low_memory_reduction_policy](../reference/kumo/make_egress_path/low_memory_reduction_policy.md),
  [no_memory_reduction_policy](../reference/kumo/make_egress_path/no_memory_reduction_policy.md) and
  options give advanced control over memory vs. spool IO trade-offs when
  available is memory low.
* New [shrink_policy](../reference/kumo/make_queue_config/shrink_policy.md)
  option to give advanced control over memory vs. spool IO trade-offs when
  messages are delayed.
* Expose `back_pressure` option to `shaping:setup_with_automation` call. This
  allows setting the underlying
  [back_pressure](../reference/kumo/configure_log_hook.md) for the TSA log
  hooks.
* local logger will now create sub-directories if they do not already exist
  below the configured log directory.
* traffic-gen: you may now specify relative weights for the randomly generated destination
  domains using eg: `--domain gmail.com:3 --domain outlook.com:1` to have gmail.com
  be 3x more likely to be generated than outlook.com.
* New [kumo.dns.set_mx_timeout](../reference/kumo.dns/set_mx_timeout.md) option
  to configure a timeout for MX record resolution. #325
* New [kumo.dns.set_mx_negative_cache_ttl](../reference/kumo.dns/set_mx_negative_cache_ttl.md)
  option to configure the duration used for caching MX resolution errors.
* New `lruttl` cache implementation exposes more internal cache metrics to
  prometheus with the `lruttl_` prefix.

## Fixes

* When using
  [kumo.dkim.set_signing_threads](../reference/kumo.dkim/set_signing_threads.md),
  some extraneous unused threads would be created.
* Using a display name with commas in the builder mode of the HTTP injection
  API would produce an invalid mailbox header.
* Potential stack overflow during spool enumeration when using
  `max_message_rate` with local (non-redis) throttles together with custom lua delivery
  handlers.
