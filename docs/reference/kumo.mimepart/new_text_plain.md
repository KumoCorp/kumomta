# new_text_plain

```lua
kumo.mimepart.new_text_plain(CONTENT)
```

{{since('dev')}}

Constructs a new [MimePart](../mimepart/index.md) with `Content-Type: text/plain`.

The `CONTENT` parameter must be a UTF-8 string.

## Example

```lua
local kumo = require 'kumo'
local part = kumo.mimepart.new_text_plain 'Hello!'
```

