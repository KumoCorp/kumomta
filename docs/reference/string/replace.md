# `kumo.string.replace(STRING, FROM, TO)`

{{since('dev')}}

Replaces all matches of `FROM` with `TO` and returns the resulting string.

```lua
assert(kumo.string.replace('this is old', 'old', 'new') == 'this is new')
```
