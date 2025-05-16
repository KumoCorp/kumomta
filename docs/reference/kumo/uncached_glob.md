---
tags:
 - utility
 - filesystem
---

# kumo.uncached_glob

```
kumo.uncached_glob(pattern [, relative_to])
```

{{since('2024.06.10-84e84b89')}}

!!! warning
    This function can cause an expensive filesystem walk to occur, especially
    if used on a storage volume that is experiencing IO pressure (such as
    from spooling or logging). You probably should use the implicitly
    cached [glob](glob.md) function instead of this one. If you must use this one,
    then it is strongly advised that you avoid calling it from the file-level
    scope of your policy scripts in order to avoid unconditionally triggering
    the walk on every lua context construction.

This function evalutes the glob `pattern` and returns an array containing the
absolute file names of the matching results.  Due to limitations in the lua
bindings, all of the paths must be able to be represented as UTF-8 or this
function will generate an error.

The optional `relative_to` parameter can be used to make the results relative
to a path.  If the results have the same prefix as `relative_to` then it will
be removed from the returned path. The default for for this parameter is `.`.

!!! note
    If the specified pattern or path references a directory that doesn't
    exist, or a directory that is inaccessible to the kumo user, no
    error will be generated; those paths are silently omitted from
    the results.

```lua
local kumo = require 'kumo'

-- logs the names of all of the '*.conf' files under `/etc`
print(kumo.json_encode_pretty(kumo.uncached_glob '/etc/*.conf'))
```

