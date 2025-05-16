# kumo.string.trim_end

```lua
kumo.string.trim_end(STRING)
```

{{since('2024.09.02-c5476b89')}}

Returns a string with trailing whitespace removed.

‘Whitespace’ is defined according to the terms of the Unicode Derived Core
Property `White_Space`, which includes newlines.

```lua
assert(kumo.string.trim_end '\n Hello\tworld\t\n' == '\n Hello\tworld')
```


