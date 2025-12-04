---
tags:
  - aaa
---

# kumo.aaa.set_check_cache_ttl

{{since('dev')}}

```lua
kumo.on('pre_init', function()
  kumo.aaa.set_check_cache_ttl '5 minutes'
end)
```

This function sets the Time To Live (TTL) duration for the ACL-check cache.

The ACL-check cache is used to cache the overall status of an authorization
check for a given `resource`, `privilege`, and `auth_info` combination.

The default duration is `5 minutes`.

!!! note
    This function is intended to be used in `pre_init` as shown above. It can be
    called at any time, but reducing the TTL after init will not automatically trigger
    a cache eviction.
