# `kumo.on('throttle_insert_ready_queue', function(message))`

{{since('dev')}}

!!! note
    This event was actually added in `2024.06.10-84e84b89` but
    with a broken event registration that prevented it from working.
    That was corrected in the version shown above.

This event is triggered when a message is ready to move from its
containing scheduled queue and into a ready queue.

Its purpose is to allow you to evaluate any throttles defined by
your policy; see [kumo.make_throttle()](../kumo/make_throttle.md) for more
information on throttles.

Multiple instances of the `throttle_insert_ready_queue` event can be registered,
and they will be called in the order in which they were registered,
until all registered events are called, or until one explicitly
returns `nil` to signal that no more should be triggered.

The example below will limit each tenant to send no more than `1000` messages
per hour:

```lua
kumo.on('throttle_insert_ready_queue', function(msg)
  -- limit each tenant to 1000/hr
  local tenant = msg:get_meta 'tenant'
  local throttle = kumo.make_throttle(
    string.format('tenant-send-limit-%s', tenant),
    '1000/hr'
  )
  throttle:delay_message_if_throttled(msg)
end)
```

This example allows each tenant to have an individual limit; you could
load the limits from a data file or database if you prefer.

```lua
local function per_tenant_throttle(tenant_name)
  -- default to 1000/hr unless otherwise overridden
  local rate = '1000/hr'
  if tenant_name == 'tenant_1' then
    -- Allow increased rate for this tenant
    rate = '10000/hr'
  end
  return kumo.make_throttle(
    string.format('tenant-send-limit-%s', tenant_name),
    rate
  )
end

kumo.on('throttle_insert_ready_queue', function(msg)
  local tenant = msg:get_meta 'tenant'
  local throttle = per_tenant_throttle(tenant)
  throttle:delay_message_if_throttled(msg)
end)
```
