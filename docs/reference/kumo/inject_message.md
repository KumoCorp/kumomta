---
tags:
 - message
---

# kumo.inject_message

```lua
kumo.inject_message(MSG)
```

{{since('dev')}}

Injects the `MSG` into the queue subsystem and begins delivering the message.

When this function returns successfully, you should consider the message that
you provided as an argument to now be owned by the core of kumomta and assume
that it may have already been delivered and removed from the spool.

This function can fail due to issues resolving queue configuration or spooling.
In error situations, an error will be raised that you can trap using the lua
`pcall` function if appropriate.

!!! danger
    Unless it is explicitly indicated in the documentation to be safe, you **MUST
    NOT** call this function on any message other than one that you have created
    via [kumo.make_message](make_message.md), otherwise you risk both duplicate
    delivery and loss of accountability for the message.

