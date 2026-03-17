---
tags:
  - xfer
---
# kumo.xfer.get_xfer_target

{{since('2026.03.04-bb93ecb1')}}

```lua
local proto = kumo.xfer.get_xfer_target(msg)
```

Returns the XferProtocol destination URL for the message, if any.

The `msg` parameter is a [Message](../message/index.md) object. If the message
is not destined for another kumomta node via the xfer protocol, then this
function will return `nil`.

