# `message:set_meta(KEY, VALUE)`

Messages are associated with some metadata. You can think of this metadata
as being equivalent to a JSON object.

The `set_meta` method allows you to set a field of that object to a value
that you specify.

You can assign any value that is serializable as a JSON:

```lua
-- set foo='bar', a string value
msg:set_meta('foo', 'bar')

-- set foo=123, a numeric value
msg:set_meta('foo', 123)

-- set foo=true, a boolean value
msg:set_meta('foo', true)

-- set foo={key="value"}, an object value
msg:set_meta('foo', { key = 'value' })
```

You can retrieve a metadata value via [message:get_meta](get_meta.md).

## Pre-defined meta values

The following meta values have meaning to KumoMTA:

* `"queue"` - specify the name of the queue to which the message will be queued. Must be a string value.
* `"tenant"` - specify the name/identifier of the tenant, if any. Must be a string value.
* `"campaign"` - specify the name/identifier of the campaign. Must be a string value.
