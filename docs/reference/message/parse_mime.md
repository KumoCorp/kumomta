# parse_mime

```lua
local mime_part = message:parse_mime()
```

{{since('dev')}}

Returns a [MimePart](../mimepart/index.md) representation of the message content.

!!! note
    While you can modify the returned mime structure, any such changes are not
    automatically reflected in the message content.  You must explicitly re-assign
    the content via `msg:set_data(tostring(mime_part))` to apply them.

## Example: attachment extraction

This demonstrates how to obtain a list of all attachments found (recursively)
in the incoming message and print out the file name, content type and the
decoded content.

This is not a useful production-ready example (you definitely don't want to log
all of the attachment content like this!) but can serve as a starting point for
policies that need to operate on attachments for scanning, compliance or
automation purposes.

```lua
kumo.on('smtp_server_message_received', function(message, conn_meta)
  local mime_part = message:parse_mime()
  local structure = mime_part:get_simple_structure()

  for _, attachment in ipairs(structure.attachments) do
    print(attachment.file_name, attachment.content_type, attachment.part.body)
  end
end)
```
