# rebuild

```lua
local rebuilt = mimepart:rebuild()
```

{{since('2025.10.06-5ec871ab')}}

`mimepart:rebuild()` will return a new, distinct, mimepart by interpreting
`mimepart` and extracting all of the key features and building up the new mime
part from that information.

This is equivalent to the "fix" performed by
[message:check_fix_conformance](../message/check_fix_conformance.md) except
that it does *NOT* modify the part in-place.

!!!warning
    Rebuilding messages with this method is inherently imperfect: it is based on a
    deliberately relaxed interpretation of the message content and it is
    possible, or even likely, that non-conforming input is parsed in a way
    that results in omitting certain details from the original input.

    The purpose of this method is as a best-effort convenience for correcting
    minor and obviously recognizable issues that cannot easily be resolved at
    the message generation stage.

    It is recommended that you carefully evaluate the effects of this method
    before deploying it in production.

The example below is purely demonstrative, and it is NOT recommended to be used
in production.  It is equivalent to the fixing portion of the example found in
[message:check_fix_conformance](../message/check_fix_conformance.md), but
unconditionally rebuilds every message.  It is almost certainly undesirable to
run this for any real workload.

```lua
kumo.on('smtp_server_message_received', function(msg)
  local mimepart = msg:parse_mime()
  msg:set_data(tostring(mimepart:rebuild()))
end)
```

