# kumo.string.trim_start

```lua
kumo.string.trim_start(STRING)
```

{{since('2024.09.02-c5476b89')}}

Returns a string with leading whitespace removed.

‘Whitespace’ is defined according to the terms of the Unicode Derived Core
Property `White_Space`, which includes newlines.

```lua
assert(kumo.string.trim_start '\n Hello\tworld\t\n' == 'Hello\tworld\t\n')
```


