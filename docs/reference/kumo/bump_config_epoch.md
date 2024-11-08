# `kumo.bump_config_epoch()`

{{since('dev')}}

Increments the current *configuration epoch* and notifies various
internal modules and processes of the change, causing them to refresh
their state.

See [Configuration Monitoring](../configuration.md#configuration-monitoring)
for more information on this topic.

This particular function is intended to be used in a reactive manner. For
example, the shaping helper uses [kumo.spawn_task](spawn_task.md) and
[kumo.http.connect_websocket](../kumo.http/connect_websocket.md) to subscribe to
the TSA daemon, and then calls `kumo.bump_config_epoch()` in response to
changes in configuration provides by TSA daemon.
