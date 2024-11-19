# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* Log records now include an optional `session_id` field to correlate
  messages received or sent (depending on the type of the record) on
  the same connection/session. #316

## Fixes

* When `enable_tls` is set to `Required` or `RequiredInsecure`, ignore the
  effect of `remember_broken_tls`.  This makes it easier to set a default value
  of `remember_broken_tls` without having to remember to special case it for
  sites where TLS is required, or sites that use MTA-STS or DANE to advertise
  that it should be required.
