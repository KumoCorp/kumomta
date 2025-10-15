# kumo.file_type.from_bytes

```lua
local ft = kumo.file_type.from_bytes(BYTES)
```

{{since('dev')}}

Attempts to determine the file type from the provided string, which may also be
binary bytes.

This function will always succeed in returning a guess, which may or may not be
accurate.

The return value is a [FileTypeResult](index.md#filetyperesult)

```lua
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local ft = kumo.file_type.from_bytes '\xCA\xFE\xBA\xBE'
utils.assert_eq(ft, {
  name = 'Java class file',
  extensions = { 'class' },
  media_types = {
    'application/java',
    'application/java-byte-code',
    'application/java-vm',
    'application/x-httpd-java',
    'application/x-java',
    'application/x-java-class',
    'application/x-java-vm',
  },
})
```
