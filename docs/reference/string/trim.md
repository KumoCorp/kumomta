# kumo.string.trim

```lua
kumo.string.trim(STRING)
```

{{since('2024.09.02-c5476b89')}}

Returns a string with leading and trailing whitespace removed.

‘Whitespace’ is defined according to the terms of the Unicode Derived Core
Property `White_Space`, which includes newlines.

```lua
assert(kumo.string.trim '\n Hello\tworld\t\n' == 'Hello\tworld')
```

