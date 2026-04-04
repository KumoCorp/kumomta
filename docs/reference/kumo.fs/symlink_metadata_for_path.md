---
tags:
 - utility
 - filesystem
---

# kumo.fs.symlink_metadata_for_path

```lua
kumo.fs.symlink_metadata_for_path(PATH)
```

{{since('dev')}}

This function behaves exactly like [metadata_for_path](metadata_for_path.md),
except that it does not follow symbolic links and instead returns information
about the symbolic link itself.


```lua
local kumo = require 'kumo'

local ok, metadata = pcall(kumo.fs.symlink_metadata_for_path, '/tmp/testfile')
if ok then
  print('mtime', metadata.mtime.rfc2822)
else
  print('error', metadata)
end
```

