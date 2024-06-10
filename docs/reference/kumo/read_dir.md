---
tags:
 - utility
 - filesystem
---

# `kumo.read_dir(path)`

{{since('2024.06.10-84e84b89')}}

This function returns an array containing the absolute file names of the
directory specified.  Due to limitations in the lua bindings, all of the paths
must be able to be represented as UTF-8 or this function will generate an
error.

```lua
local kumo = require 'kumo'

-- logs the names of all of the entries under `/etc`
print(kumo.json_encode_pretty(kumo.read_dir '/etc'))
```

