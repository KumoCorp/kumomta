---
tags:
 - utility
 - filesystem
---

# kumo.glob

```
kumo.glob(pattern [, relative_to, [, ttl_seconds]])
```

{{since('2024.06.10-84e84b89')}}

This function evalutes the glob `pattern` and returns an array containing the
absolute file names of the matching results.  Due to limitations in the lua
bindings, all of the paths must be able to be represented as UTF-8 or this
function will generate an error.

The optional `relative_to` parameter can be used to make the results relative
to a path.  If the results have the same prefix as `relative_to` then it will
be removed from the returned path. The default for this parameter is `.`.

The optional `ttl_seconds` parameter specifies how long the results of
the glob operation will be cached.  Subsequent calls to `glob` with the
same `pattern` and `relative_to` will return those previously cached
results until the TTL expires.  The default TTL is `60` seconds.

!!! warning
    This function can cause an expensive filesystem walk to occur, especially
    if used on a storage volume that is experiencing IO pressure (such as
    from spooling or logging). Take care to scope the pattern to minimize
    the impact of the walk, and the `ttl_seconds` parameter to something that
    is appropriate to your use case.

!!! note
    If the specified pattern or path references a directory that doesn't
    exist, or a directory that is inaccessible to the kumo user, no
    error will be generated; those paths are silently omitted from
    the results.

```lua
local kumo = require 'kumo'

-- logs the names of all of the '*.conf' files under `/etc`
print(kumo.json_encode_pretty(kumo.glob '/etc/*.conf'))
```


