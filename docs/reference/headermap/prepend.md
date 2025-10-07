# prepend

```lua
headers:prepend(NAME, VALUE)
```

{{since('2025.10.06-5ec871ab')}}

Constructs a new header with `NAME` and `VALUE` and prepends it to the header map.

```lua
headers:prepend('X-Something', 'Some value')
```
