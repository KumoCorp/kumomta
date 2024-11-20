# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* Log records now include an optional `session_id` field to correlate
  messages received or sent (depending on the type of the record) on
  the same connection/session. #316

* Updated embedded libunbound to 1.22

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
