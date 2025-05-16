# set_data

```lua
message:set_data(payload)
```

{{since('2024.06.10-84e84b89')}}

Replaces the message body/data completely.
It is your responsibility to ensure that the content is well-formed, has
canonical CRLF line endings, and uses appropriate transfer-encoding, otherwise
the system will misbehave when delivering the message.

See also:
* [msg:get_data()](get_data.md)
