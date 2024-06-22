# `kumo.string.trim(STRING)`

{{since('dev')}}

Returns a string with leading and trailing whitespace removed.

‘Whitespace’ is defined according to the terms of the Unicode Derived Core
Property `White_Space`, which includes newlines.

```lua
assert(kumo.string.trim '\n Hello\tworld\t\n' == 'Hello\tworld')
```

