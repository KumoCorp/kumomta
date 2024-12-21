# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* Log records now include an optional `session_id` field to correlate
  messages received or sent (depending on the type of the record) on
  the same connection/session. #316

* Updated embedded libunbound to 1.22

* Use more compact representation of ResolvedAddress in logs. Instead of
  showing something like `ResolvedAddress { name: "some.host.", addr: 10.0.0.1 }`
  we now display it as `some.host./10.0.0.1` which is a bit easier to
  understand and occupies less space in the logs.

* We will now trigger the
  [requeue_message](../reference/events/requeue_message.md) event in the
  case of an error resolving the ready queue, such as DNS related errors.
  This gives an opportunity to rebind or reject messages which are
  experiencing persistent DNS resolution issues. #319

* New `dns_mx_resolve_in_progress`, `dns_mx_resolve_status_ok`,
  `dns_mx_resolve_status_fail`, `dns_mx_resolve_cache_hit`,
  `dns_mx_resolve_cache_miss` metrics that reflect the status of MX
  resolution. These are available via the metrics endpoints.

* [maildir](../reference/kumo/make_queue_config/protocol.md#specifying-directory-and-file-modes-for-maildir)
  can now specify file and directory modes. #109

* [maildir](../reference/kumo/make_queue_config/protocol.md#advanced-maildir-path)
  now supports template expansion in the `maildir_path`. #109

* TSA now supports `"Bounce"`, `"BounceTenant"` and `"BounceCampaign"` actions
  which create bounces for scheduled queues which match the
  domain/tenant/campaign of the triggering event. #272

* Ready Queues now use intrusive lists through the internal Message structure,
  which keeps memory usage for the ready queues bounded to `O(number-of-messages)`
  rather than the previous `O(number-of-ready-queues * max_ready)`.

* Provider match rules now also support exactly matching MX hostnames via the
  new `HostName` entry.

* kcli queue-summary will now show connection limit and connection rate throttled
  status effects as part of the ready queue information, making it easier to
  determine when a (potentially shared with multiple nodes) limit might be
  responsible for messages in the ready queue. There is a corresponding
  [ready-q-states](../reference/rapidoc.md/#get-/api/admin/ready-q-states/v1) API
  endpoint that can be used to retrieve this same information.


## Fixes

* When `enable_tls` is set to `Required` or `RequiredInsecure`, ignore the
  effect of `remember_broken_tls`.  This makes it easier to set a default value
  of `remember_broken_tls` without having to remember to special case it for
  sites where TLS is required, or sites that use MTA-STS or DANE to advertise
  that it should be required.

* When configuring the unbound resolver, the port number was not passed through
  for the upstream DNS server, so non-standard ports would not be respected.
  #314

* Expiration was checked only when incrementing the number of attempts, or when
  spooling in.  There are some situations where a message can be delayed and
  re-queued without incrementing the number of attempts, which meant that some
  messages could linger in the queues until they are actually attempted again.

* Running `validate-shaping` without passing any files to validate would report
  `OK` instead of telling you that you should have passed one or more file names.

* Some messages could sometimes get delayed slightly longer than intended when
  using TimerWheels (the default) when moving from the scheduled queue to
  the ready queue.

* The `get_listener_domain` event handler results were incorrectly cached globally
  which allowed a given source IP which successfully authenticated in one session
  to appear authenticated for other separate connections made by that *same IP*
  to/from the same domain, within the same 60 second period. #320

* TSA daemon would not report the list of scheduled queue suspensions in the
  initial websocket request made by a (re)connecting client.

* Certain providers configurations with multiple `MXSuffix` rules and multiple
  candidate MX hosts might not match in cases where they should.

* Connection establishment rate for a ready queue could be constrained to
  1-new-connection-per-10-minute period if that ready queue had no new messages
  being added to it and if a connection limit had prevented new connections
  being opened.
