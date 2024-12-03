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
