# get_simple_structure

```lua
local structure = mimepart:get_simple_structure()
```

{{since('dev')}}

A MIME message is an encoding of a tree of pieces of content which makes things
relatively complex to script around in various processing work flows,
especially because there can be multiple ways to encode similar information
that result in different tree structures.

This method will walk the tree structure starting from `mimepart` and collect
together the main points of interest, returning a lua table holding that
simplified and flattened view.  The following fields are present in the
simplified structure:

  * `text_part` - The first `text/plain` [MimePart](index.md) found in the walk, if any.
  * `html_part` - The first `text/html` [MimePart](index.md) found in the walk, if any.
  * `header_part` - The "main" part from the perspective of header analysis
  * `attachments` - An array style table holding a list of all the attachments.

Each attachment table entry has the following fields:

  * `file_name` - The suggested name to use when saving the attachment. If the
    `Content-Disposition` header defined the file name, then that will be used.
    Otherwise, a name will be synthesized based on the position of the
    attachment within the MIME tree and will look something like `attachment1`
    or `attachment2.3`.
  * `inline` - will be `true` if the attachment was marked as having an inline
    disposition, `false` otherwise.
  * `content_id` - if the `Content-ID` header is defined, this field will hold
    its value.
  * `part` - the [MimePart](index.md) for the attachment.  You can use this to
    access its body (eg: [mimepart.body](body.md) or headers.

## Example of modifying incoming message content

This example prepends text to both the text and html parts of incoming messages:

```lua
kumo.on('smtp_server_message_received', function(message, conn_meta)
  local mime_part = message:parse_mime()
  local structure = mime_part:get_simple_structure()

  if structure.text_part then
    structure.text_part.body = 'PREPENDED!\r\n' .. structure.text_part.body
  end

  if structure.html_part then
    structure.html_part.body = '<B>PREPENDED!</B>\r\n'
      .. structure.html_part.body
  end

  -- Apply the changed content to the message
  message:set_data(tostring(mime_part))
end)
```

## Example of dumping incoming attachments

This example logs the attachment information and contents during reception.
It is not recommended for production workflows, as the contents can be
large and unsuitable for capture in the diagnostic log, but can be helpful
in some debugging scenarios.

```lua
kumo.on('smtp_server_message_received', function(message, conn_meta)
  local mime_part = message:parse_mime()
  local structure = mime_part:get_simple_structure()

  for _, attachment in ipairs(structure.attachments) do
    print(attachment.file_name, attachment.content_type, attachment.part.body)
  end
end)
```
