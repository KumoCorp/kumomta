---
tags:
  - xfer
---
# kumo.xfer.cancel_xfer

{{since('dev')}}

```lua
kumo.xfer.cancel_xfer(msg, opt_reason)
```

Cancels any xfer routing that might be applied to `msg` (which must be a
[Message](../message/index.md) object).

If the message is not configured to xfer then this function will not make any
changes to the message.

If `opt_reason` is specified, it will be used as the reason string in an
`AdminRebind` log record to capture any queue change that might occur due to
calling this function.

If `opt_reason` is not specified (omitted, or is explicitly set to `nil`),
then no `AdminRebind` log entry will be produced.



