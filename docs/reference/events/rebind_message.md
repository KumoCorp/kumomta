# `kumo.on('rebind_message', function(message, data))`

{{since('2024.09.02-c5476b89')}}

This event is triggered when processing a rebind request that was triggered by
the [/api/admin/rebind/v1](../rapidoc.md/#post-/api/admin/rebind/v1) HTTP endpoint
(which is is what is used by the `kcli rebind` command).

This event is only triggered when `trigger_rebind_event` is set in the incoming
request.

The purpose of the event is for you to perform an optional, site-specific
modification to the message and/or its metadata in response to the rebind
request.

The `data` parameter is the verbatim `data` field from the request.

```lua
kumo.on('rebind_message', function(msg, data)
  if some_condition then
    msg:set_meta('queue', data.queue)
  end
end)
```

