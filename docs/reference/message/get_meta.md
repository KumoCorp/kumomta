---
tags:
 - meta
---

# get_meta

```lua
message:get_meta(KEY)
```

Messages are associated with some metadata. You can think of this metadata
as being equivalent to a JSON object.

The `get_meta` method allows you to retrieve a field of that object.

```lua
msg:set_meta('foo', 'bar')
print(msg:get_meta 'foo') -- prints 'bar'
```

See also [msg:set_meta](set_meta.md).

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
