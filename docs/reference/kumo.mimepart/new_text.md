# new_text

```lua
kumo.mimepart.new_text(CONTENT_TYPE, CONTENT)
```

{{since('2025.10.06-5ec871ab')}}

Constructs a new [MimePart](../mimepart/index.md) with the `Content-Type`
header set to `CONTENT_TYPE`.

The `CONTENT` parameter must be a UTF-8 string.

## Example

```lua
local kumo = require 'kumo'
local part = kumo.mimepart.new_text('text/markdown', 'Some markdown text')
```


