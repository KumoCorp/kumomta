# `kumo.string.replace(STRING, FROM, TO)`

{{since('2024.09.02-c5476b89')}}

Replaces all matches of `FROM` with `TO` and returns the resulting string.

```lua
assert(kumo.string.replace('this is old', 'old', 'new') == 'this is new')
```
