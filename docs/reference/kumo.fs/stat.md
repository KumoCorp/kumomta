---
tags:
 - utility
 - filesystem
---

# kumo.fs.stat

```lua
kumo.fs.stat(path)
```

{{since('dev')}}

* `path` same as supplied path
* `is_file` returns true if path points to a file
* `is_dir` returns true if path points to a directory
* `is_symlink` returns true if path is for a symbolic link
* `len` size of the file
* `readonly` returns true if path permission is set as readonly

The following fields may not be available on all platforms, in such case it will return nil.

* `mtime` last modification time represented in `Time` object
* `atime` last access time represented in `Time` object
* `ctime` creation time represented in `Time` object

```lua
local kumo = require 'kumo'

local ok, stat = pcall(kumo.fs.stat, '/tmp/testfile')
if ok then
  print('mtime', stat.mtime.rfc2822)
else
  print('stat error', stat)
end
```

