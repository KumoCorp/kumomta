---
tags:
 - meta
---

# set_meta

```lua
message:set_meta(KEY, VALUE)
```

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

## Naming convention for user-defined keys

KumoMTA reserves a set of predefined meta keys (see [Predefined Metadata](../metadata.md)) and may add more over time. To avoid collisions with current and future predefined keys, prefix your own meta keys with `x_`.

This aligns with the `x_*` keys produced by
[message:import_x_headers](import_x_headers.md) and the default
`snake_case` transform used by [message:import_headers](import_headers.md),
and KumoMTA's own predefined meta keys will never start with `x_`.

```lua
-- recommended for application-specific data
msg:set_meta('x_campaign_id', '12345')
msg:set_meta('x_my_app_flag', true)
```

## Pre-defined meta values

The following meta values are unique to the message scope:

<style>
table tbody tr td:nth-of-type(2) {
  white-space: nowrap;
}
</style>

|Scope|Name|Purpose|Since|
|----|----|-------|-----|
|Message|`queue`|specify the name of the queue to which the message will be queued. Must be a string value.||
|Message|`tenant`|specify the name/identifier of the tenant, if any. Must be a string value.||
|Message|`campaign`|specify the name/identifier of the campaign. Must be a string value.||
|Message|`routing_domain`|Overrides the domain of the recipient domain for routing purposes.|{{since('2023.08.22-4d895015', inline=True)}}|

!!! Note
    Additional metadata is available in the message scope that is copied in from the connection scope, for a full list of all available metadata, see the [Predefined Metadata](../metadata.md) page.
