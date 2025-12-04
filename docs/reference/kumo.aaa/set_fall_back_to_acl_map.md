---
tags:
  - aaa
---

# kumo.aaa.set_fall_back_to_acl_map

{{since('dev')}}

```lua
kumo.on('pre_init', function()
  kumo.aaa.set_fall_back_to_acl_map(true)
end)
```

This function accepts a single `boolean` argument that controls whether or not
the default ACL map will be used as a fallback source for an ACL definition if
the [get_acl_definition](../events/get_acl_definition.md) event doesn't return
an ACL.

The default is `true`.

If you set this to `false` then your `get_acl_definition` implementation is
considered to the definitive and only source of ACL definitions in your deployment,
completely replacing the product default ACL definitions.

!!! note
    This function is intended to be used in `pre_init` as shown above. It can be
    called at any time, but it is recommended that you call it within `pre_init`
    and restart the service if you want to change its value, in order for your
    overall AAA configuration to apply consistently.
