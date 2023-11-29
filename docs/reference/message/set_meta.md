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
* `"authz_id"` - the authorization id if the message was received via authenticated SMTP
* `"authn_id"` - the authentication id if the message was received via authenticated SMTP
* `"reception_protocol"` - `"ESMTP"` or `"HTTP"`
* `"received_via"` - the address:port of the local machine which received the message. Currently only set for SMTP receptions.
* `"received_from"` - the address:port of the peer address from which we received the message
* `"routing_domain"` - {{since('2023.08.22-4d895015', inline=True)}}. Overrides the domain of the recipient domain for routing purposes.
* `"hostname"` - {{since('2023.11.28-b5252a41', inline=True)}}. A copy of the effective value of the hostname set by [kumo.start_esmtp_listener](../kumo/start_esmtp_listener.md#hostname)
