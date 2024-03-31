# `message:append_text_plain(content)`

{{since('dev')}}

Identifies the primary `text/plain` part of the message, decodes its transfer
encoding, and appends the `content` string to it. The part is then
re-transfer-encoded and the message data is updated.

* See also:
* [msg:set_data()](set_data.md)
* [msg:append_text_html()](append_text_html.md)

