---
tags:
  - aaa
---

# kumo.aaa.set_acl_cache_ttl

{{since('dev')}}

```lua
kumo.on('pre_init', function()
  kumo.aaa.set_acl_cache_ttl '5 minutes'
end)
```

This function sets the Time To Live (TTL) duration for the ACL-definition cache.

The ACL-definition cache is used to cache the result of the
[get_acl_definition](../events/get_acl_definition.md) event callback for a
given resource.

The default duration is `5 minutes`.

!!! note
    This function is intended to be used in `pre_init` as shown above. It can be
    called at any time, but reducing the TTL after init will not automatically trigger
    a cache eviction.
