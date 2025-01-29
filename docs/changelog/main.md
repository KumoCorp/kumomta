# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* Added [kcli inspect-sched-q](../reference/kcli/inspect-sched-q.md) command. #231
* Added
  [reconnect_strategy](../reference/kumo/make_egress_path/reconnect_strategy.md)
  egress path option to control what happens with a session that experiences
  a disconnection during message sending.
* Improved robustness of [kcli
  trace-smtp-server](../reference/kcli/trace-smtp-server.md) and [kcli
  trace-smtp-client](../reference/kcli/trace-smtp-client.md) on busy servers.
* kafka client can now process non-UTF8 binary payloads. Thanks to @cai-n! #328
* Exposed the `max_burst` throttle parameter to the throttle parser. This
  functionality was always present, it just wasn't configurable. See the
  [kumo.make_throttle](../reference/kumo/make_throttle.md) docs for the revised
  throttle specification.

## Fixes

* Regression with the recent RSET optimizations: we didn't issue an RSET if a send
  failed partway through, leading to issues with the connection state.
* SMTP Client could sometimes get stuck attempting to process a series of messages
  on a connection that had previously been closed.
* Potential cache thrashing issue with `remember_broken_tls` could lead to a larger
  number of connection attempts to sites with broken TLS.
