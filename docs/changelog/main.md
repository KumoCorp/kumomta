# Unreleased Changes in The Mainline

## Breaking Changes

* Handling of egress source/pool has moved from `define_egress_source` and `define_egress_pool` to `make_egress_source` and `make_egress_pool`, allowing these configurations to be loaded dynamically at runtime, removing the need for a server restart. See [make_egress_source](../userguide/configuration/sendingips.md).

* Configuration of relay domains has been moved out of `start_esmtp_listener` into its own event, allowing these configurations to be loaded dynamically at runtime, removing the need for a server restart. See [make_listener_domain](../userguide/configuration/smtplisteners.md).

## Enhancements

* [spool_message_enumerated](../reference/events/spool_message_enumerated.md)
  event. #42
* [Rabbit MQ/AMQP Event/Message Publishing](../userguide/policy/amqp.md). [#31](https://github.com/KumoCorp/kumomta/issues/31)
* [SOCKS5 Proxy Support](../userguide/operation/proxy.md). [#45](https://github.com/KumoCorp/kumomta/issues/45)
* Added helper policy scripts for managing egress source/pool and listeners domains. See [make_egress_source](../userguide/configuration/sendingips.md) and [make_listener_domain](../userguide/configuration/smtplisteners.md).

## Fixes

* Fix issue with log flushing during shutdown. [#46](https://github.com/KumoCorp/kumomta/issues/46)