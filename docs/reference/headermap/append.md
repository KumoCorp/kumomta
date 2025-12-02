# append

```lua
headers:append(NAME, VALUE)
```

{{since('2025.12.02-67ee9e96')}}

Constructs a new header with `NAME` and `VALUE` and appends it to the header map.

If the header value is ascii then it will be soft wrapped at whitespace around
75 columns, and hard-wrapped regardless of whitespace at 900 columns.

If the header value is non-ascii then it will be quoted printable encoded using
RFC 2047 header encoding.

```lua
headers:append('X-Something', 'Some value')
```


