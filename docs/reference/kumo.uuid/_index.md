# Module `kumo.uuid`

This module provides functions for parsing and generating UUIDs.

## The UUID Object

The functions in this module return a `Uuid` object.

Printing or otherwise explicitly converting a `Uuid` object
as a string will produce the the hyphenated form of the uuid.

The following fields are available to return the bytes encoded
in various ways.

* `bytes` - returns the data as a binary byte string. This is the most compact representation, but is difficult to pass into other systems without encoding in some way. Case sensitive.
* `hyphenated` - returns the data encoded as lowercase hexadecimal with the elements of the UUID separated by hyphens. Case insensitive. Example: `69994630-3e27-11ef-91fc-cc28aa0a5c5a`
* `simple` - returns the data encoded as lowercase hexadecimal with no separating hyphens. Case insensitive. Example: `699946303e2711ef91fccc28aa0a5c5a`.
* `braced` - returns the data encoded as lowercase hexadecimal with the elements of the UUID separated by hyphens, all enclosed in curly braces. Case insensitive. Example: `{69994630-3e27-11ef-91fc-cc28aa0a5c5a}`
* `urn` - returns the data formatted as an URN. Example: `urn:uuid:69994630-3e27-11ef-91fc-cc28aa0a5c5a`.

```lua
local u = kumo.uuid.parse '69994630-3e27-11ef-91fc-cc28aa0a5c5a'
assert(tostring(u) == '69994630-3e27-11ef-91fc-cc28aa0a5c5a')
assert(u.hyphenated == '69994630-3e27-11ef-91fc-cc28aa0a5c5a')
assert(u.simple == '699946303e2711ef91fccc28aa0a5c5a')
assert(u.braced == '{69994630-3e27-11ef-91fc-cc28aa0a5c5a}')
assert(u.urn == 'urn:uuid:69994630-3e27-11ef-91fc-cc28aa0a5c5a')
assert(
  u.bytes
    == '\x69\x99\x46\x30\x3e\x27\x11\xef\x91\xfc\xcc\x28\xaa\x0a\x5c\x5a'
)
```

## Available Functions
