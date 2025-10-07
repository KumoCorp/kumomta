---
tags:
 - utility
 - filesystem
---

# kumo.fs.read_dir

```lua
kumo.fs.read_dir(path)
```

{{since('2025.10.06-5ec871ab')}}

!!! note
    In earlier versions of kumo, this function is available via the
    deprecated alias [kumo.read_dir](../kumo/read_dir.md)

This function returns an array containing the absolute file names of the
directory specified.  Due to limitations in the lua bindings, all of the paths
must be able to be represented as UTF-8 or this function will generate an
error.

```lua
local kumo = require 'kumo'

-- logs the names of all of the entries under `/etc`
print(kumo.json_encode_pretty(kumo.fs.read_dir '/etc'))
```

