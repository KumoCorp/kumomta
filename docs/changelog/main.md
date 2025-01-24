# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* Added [kcli inspect-sched-q](../reference/kcli/inspect-sched-q.md) command. #231
* Added
  [reconnect_strategy](../reference/kumo/make_egress_path/reconnect_strategy.md)
  egress path option to control what happens with a session that experiences
  a disconnection during message sending.

## Fixes

* Regression with the recent RSET optimizations: we didn't issue an RSET if a send
  failed partway through, leading to issues with the connection state.
* SMTP Client could sometimes get stuck attempting to process a series of messages
  on a connection that had previously been closed.
