# new_binary

```lua
kumo.mimepart.new_binary(CONTENT_TYPE, CONTENT, OPTIONAL_ATTACHMENT_OPTIONS)
```

{{since('dev')}}

Constructs a new [MimePart](../mimepart/index.md) for binary content.

You must provide the content type and the content itself; the content can be
any binary lua string (including strings that are actually UTF-8 text).

The MimePart will use appropriate transfer encoding for the binary data.

You may optionally specify parameters that will affect how the part will appear
when used as an attachment; if you don't care about these, you can omit the
third parameter, or pass `nil`.  If you do want to specify them, then you can
pass a lua table that allows for the following fields, which influence the `Content-Disposition` header in the resulting mime part:

  * `file_name` - an optional string to use to define the attachment file name.
  * `inline` - an optional boolean that indicates whether the attachment will be marked as being an inline attachment. The default is `false`.
  * `content_id` - an optional string that can be used to define the
    `Content-Id` header for the mime part, which is useful when generating HTML
    content that references an attachment.

## Example

```lua
local kumo = require 'kumo'
local part =
  kumo.mimepart.new_binary('application/octet-stream', '\xbb\xaa', {
    file_name = 'binary.dat',
  })
```
