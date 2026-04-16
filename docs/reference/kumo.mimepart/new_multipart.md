# new_multipart

```lua
kumo.mimepart.new_multipart(CONTENT_TYPE, PARTS, OPTIONAL_BOUNDARY)
```

{{since('2025.10.06-5ec871ab')}}

Constructs a new multi-part [MimePart](../mimepart/index.md) with the
`Content-Type` header set to `CONTENT_TYPE`, which is expected to have a
`multipart/` prefix, although any content type for which multipart semantics
are expected by consumers is permitted.

The `PARTS` parameter is an array style table containing the set of
[MimePart](../mimepart/index.md) objects that will form the children of the
newly created part.

The `OPTIONAL_BOUNDARY` parameter is an optional string that can be used to
define the MIME boundary for the various parts; you don't normally need to
specify this as the default behavior is to generate a UUID to form the boundary
string.  You might wish to set the boundary if you are producing tests and need
to make assertions on the resulting message content.

## Example

This example shows how to produce a simple message with an attachment:

```lua
local kumo = require 'kumo'

local main =
  kumo.mimepart.new_text_plain 'Hello, I am the main message content'
local attachment =
  kumo.mimepart.new_binary('application/octet-stream', '\xbb\xaa', {
    file_name = 'binary.dat',
  })

local message =
  kumo.mimepart.new_multipart('multipart/mixed', { main, attachment })

print(message)
```

That will output a message looking something like this; the boundary will vary each time:

```
Content-Type: multipart/mixed;
 boundary="S53NSUR9QJam33WHBKAceA"

--S53NSUR9QJam33WHBKAceA
Content-Type: text/plain;
 charset="us-ascii"

Hello, I am the main message content
--S53NSUR9QJam33WHBKAceA
Content-Type: application/octet-stream
Content-Transfer-Encoding: base64
Content-Disposition: attachment;
 filename="binary.dat"

u6o=
--S53NSUR9QJam33WHBKAceA--

```

