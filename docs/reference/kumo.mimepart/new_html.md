# new_html

```lua
kumo.mimepart.new_html(CONTENT)
```

{{since('dev')}}

Constructs a new [MimePart](../mimepart/index.md) with `Content-Type: text/html`.

The `CONTENT` parameter must be a UTF-8 string.

## Example

```lua
local kumo = require 'kumo'
local part = kumo.mimepart.new_html '<b>Hello</b>!'
```
