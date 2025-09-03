# prepend

```lua
headers:prepend(NAME, VALUE)
```

{{since('dev')}}

Constructs a new header with `NAME` and `VALUE` and prepends it to the header map.

```lua
headers:prepend('X-Something', 'Some value')
```
