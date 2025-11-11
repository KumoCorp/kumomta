# prepend_header

```lua
message:prepend_header(NAME, VALUE)
```

Constructs a header from `NAME: VALUE` and prepends it to the message content.

The `VALUE` is taken as-is and used as the header value.

{{since('dev')}}

This method now accepts an additional optional `ENCODE` parameter, which should
be a boolean value:

```lua
message:prepend_header(NAME, VALUE, ENCODE)
```

When `ENCODE` is set to true then the `VALUE` will be encoded:

* If the header value is ascii then it will be soft wrapped at whitespace
  around 75 columns, and hard-wrapped regardless of whitespace at 900 columns.
* If the header value is non-ascii then it will be quoted printable encoded
  using RFC 2047 header encoding.

