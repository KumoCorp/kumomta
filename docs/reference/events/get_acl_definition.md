---
tags:
  - aaa
---

# get_acl_definition

```lua
kumo.on('get_acl_definition', function(resource) end)
```

{{since('dev')}}

This event is called when the system is performing an authorization check.  Its
purpose is to provide the definition of an Access Control List (ACL) for the
requested *resource*.

If the event returns `nil`, or doesn't return any explicit value, or is not
defined, then the default behavior will be to consider the product default ACL
Map and use the definition provided by that map, if any.

## Augmenting the default ACL

The following example shows how you might define your own augmented ACL
definitions in a file named `/opt/kumomta/etc/custom_acl.toml` that will be
refreshed every 5 minutes (to pick up any changes):

```lua
local get_acl_map = kumo.memoize(function()
  return kumo.aaa.load_acl_map '/opt/kumomta/etc/custom_acl.toml'
end, {
  name = 'acl_map_cache',
  ttl = '5 minutes',
  capacity = 4,
})

kumo.on('get_acl_definition', function(resource)
  return get_acl_map():get(resource)
end)
```

Entries that you define in your custom ACL file will take the place of entries
in the product default ACL for a given *resource*; there is no merging of
entries.

See also: [kumo.aaa.load_acl_map](../kumo.aaa/load_acl_map.md).

## Replacing the default ACL

The following is essentially the same as the example above, except that
it disables the fall back to the product default ACL map

```lua
local get_acl_map = kumo.memoize(function()
  return kumo.aaa.load_acl_map '/opt/kumomta/etc/custom_acl.toml'
end, {
  name = 'acl_map_cache',
  ttl = '5 minutes',
  capacity = 4,
})

kumo.on('get_acl_definition', function(resource)
  return get_acl_map():get(resource)
end)

kumo.on('pre_init', function()
  -- the `custom_acl.toml` is now considered to be the exhaustive,
  -- definitive source of ACL definitions.  The product default
  -- definitions will not be considered
  kumo.aaa.set_fall_back_to_acl_map(false)
end)
```

See also: [kumo.aaa.load_acl_map](../kumo.aaa/load_acl_map.md).

## Advanced ACL Building

You don't have to load ACLs using the TOML file syntax, you can
instead build up rules entirely in lua code.

The following example builds an ACL for a hypothetical `/api/admin/baz` HTTP
endpoint:

```lua
kumo.on('get_acl_definition', function(resource)
  if resource == 'http_listener/*/api/admin/baz' then
    return kumo.aaa.make_access_control_list {
      {
        criteria = {
          Identity = { Group = 'kumomta:http-listener-trusted-ip' },
        },
        privilege = 'GET',
        access = 'Allow',
      },
    }
  end
  -- Use the product default for everything else
  return nil
end)
```

See also: [kumo.aaa.make_access_control_list](../kumo.aaa/make_access_control_list.md).
