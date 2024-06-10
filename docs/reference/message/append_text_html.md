# `message:append_text_html(content)`

{{since('2024.06.10-84e84b89')}}

Identifies the primary `text/html` part of the message, decodes its transfer
encoding, and locates the closing `"</body>"` or `"</BODY>"` tag. If the tag
is found, `content` is inserted ahead of it. If no body tag is found, appends the
`content` string to the part. The part is then re-transfer-encoded and the
message data is updated.

This is useful for example to add a tracking pixel into the message:

```lua
umo.on('smtp_server_message_received', function(msg)
  local my_tracking_link =
    '<img src="http://10.0.0.1/img_tracker.jpg" alt="open tracking pixel">'
  msg:append_text_html(my_tracking_link)
end)
```

* See also:
* [msg:set_data()](set_data.md)
* [msg:append_text_plain()](append_text_plain.md)


