---
tags:
 - message
---

# kumo.make_message

```lua
MSG = kumo.make_message(SENDER, RECIPIENT, BODY)
```

{{since('dev')}}

Constructs a new [Message](../message/index.md) object with the specified `SENDER`, `RECIPIENT` and `BODY`.

`make_message` was created (in earlier versions of kumomta) primarily for
testing purposes, but is now provided for use together with
[kumo.inject_message](inject_message.md) to facilitate more advanced workflows.

`SENDER` is expected to be the envelope-from address.

`RECIPIENT` is expected to be the envelope-to address.

`BODY` is expected to be the appropriately formatted body payload; this is
usually a MIME message.  The body should be formatted using canonical CRLF line
endings.

The message created by `make_message` exists solely in memory until you
explicitly indicate that you want something to happen to it.  If you intend for
kumomta to deliver the message, you should call
[kumo.inject_message](inject_message.md) to enqueue the message for delivery.

