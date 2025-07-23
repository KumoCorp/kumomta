---
tags:
 - utility
 - filesystem
---

# kumo.fs.open

```
FILE = kumo.fs.open(FILENAME, OPT_MODE)
```

{{since('dev')}}

This function is similar to the lua builtin `io.open` function in that
it can be used to open a file on the filesystem either for reading or writing.
It differs from the builtin lua function in two key ways:

 * `kumo.fs.open` and the `async-file` object that it returns will not block
   the async runtime used in kumomta, whereas the builtin lua functions *will*
   block.

 * The `async-file` read and write methods are simpler and operate only on
   buffers of bytes; you must manually format or parse the buffers that are
   passed to it.

The `FILENAME` parameter is a string containing the path to the file that is to
be opened.

The `OPT_MODE` parameter is a string describing how the file should be opened.
If it is not specified, it is assumed to have the value `"r"`.  The supported values are:

 * `"r"` or `"rb"`: open the file for read only
 * `"w"` or `"wb"`: open the file for write only. The file will be created if
   it doesn't already exist
 * `"a"` or `"ab"`: open the file for write only, in append mode. The file will
   be created if it doesn't already exist.
 * `"r+"` or `"r+b"`: open the file for read and write.  The file will be
   created if it doesn't already exist. The contents of the file will be
   preserved.
 * `"w+"` or `"w+b"`: open the file for read and write.  The file will be
   created if it doesn't already exist. The contents of the file will be
   truncated.
 * `"a+"` or `"a+b"`: open the file for read and write.  The file will be
   created if it doesn't already exist. The contents of the file will be
   preserved.  Writes will only occur at the end of the file.

If the file cannot be opened, an error will be raised.

On success, returns an `async-file` object that supports the methods shown below.

```lua
local kumo = require 'kumo'

local file = kumo.fs.open('/tmp/somefile.txt', 'w')
file:write 'hello there'
file:seek 'set'
assert(file:read() == 'hello there')
```

### asyncfile:close

```lua
file:close()
```

Closes the file, releasing its resources.  This will happen implicitly when the
file object is garbage collected, but it can be hard to determine exactly when
that might happen, so it is often good practice to explicitly close it.

Explicitly calling `file:close` implicitly calls `file:flush`.

### asyncfile:flush

```lua
file:flush()
```

Flushes any buffered data to the file.

### asyncfile:read

```lua
BUFFER = file:read(OPT_SIZE)
```

Reads data from the file.  `OPT_SIZE` is an optional integer specifying how much data to read.
If omitted, the remaining size of the file is assumed.

When specifying the size, not that the returned buffer can be smaller than the requested size,
and that a subsequent read may return additional data.

Returns a string (which may be a binary string) holding the returned buffer.

### asyncfile:write

```lua
file:write(BUFFER)
```

Writes the complete contents of `BUFFER` to the file. If the write fails for
whatever reason, an error is raised.

### asyncfile:seek

```lua
POS = file:seek(OPT_WHENCE, OPT_POS)
```

Changes the current read/write position of the file, returning the new
position, implicitly flushing any buffered write if needed.

`OPT_WHENCE` describes how the position should change. If omitted, it will be assumed to be `"cur"`. The possible values are:

 * `"cur"` - compute a new position based on the current position
 * `"set"` - compute a new position based on the start of the file
 * `"end"` - compute a new position based on the end of the file

`OPT_POS` describes where to move to, relative to `OPT_WHENCE`. If omitted, it
will be assumed to be `0`.  The position must be an integer, which can be
negative.

`file:seek()` is equivalent to `file:seek('cur', 0)` which leaves the position
alone (adds `0` to the current position) and returns the current position.

`file:seek('end')` moves to the end of the file and returns the size of the file.

