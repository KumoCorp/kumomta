---
tags:
 - utility
 - filesystem
---

# kumo.fs.metadata_for_path

```lua
kumo.fs.metadata_for_path(PATH)
```

{{since('dev')}}

This function returns the file or directory attributes.
If PATH is a symbolic link, this function will traverse symbolic links and retrieves metadata of the destination file or directory.
Available attributes are as below.

Not all attributes are guaranteed to be retrievable as they may be platform dependent.

* `path` same as supplied path
* `is_file` returns true if path points to a file
* `is_dir` returns true if path points to a directory
* `is_symlink` returns true if path is for a symbolic link
* `len` size of the file
* `readonly` returns true if path permission is set as readonly

The following fields may not be available on all platforms, in such case it will return nil.

* `dev` returns the ID of the device containing the file
* `ino` returns the inode number
* `mode` returns the rights applied to this file
* `nlink` returns the number of hard links pointing to this file
* `uid` returns the user ID of the owner of this file
* `gid` returns the group ID of the owner of this file
* `rdev` returns the device ID of this file
* `size` returns the total size of this file in bytes
* `blksize` returns the block size for filesystem I/O
* `blocks` returns the number of blocks allocated to the file, in 512-byte units
* `mtime` last modification time represented in `Time` object
* `atime` last access time represented in `Time` object
* `ctime` creation time represented in `Time` object

```lua
local kumo = require 'kumo'

local ok, metadata = pcall(kumo.fs.metadata_for_path, '/tmp/testfile')
if ok then
  print('mtime', metadata.mtime.rfc2822)
else
  print('error', metadata)
end
```

