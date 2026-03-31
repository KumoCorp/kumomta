# kumo.encode.charset_decode

```lua
kumo.encode.charset_decode(CHARSET, BINARY_INPUT)
```

{{since('dev')}}

Given an the input string `BINARY_INPUT`, which is likely a binary string,
attempt to decode the string from the named `CHARSET`,
which must be one of the charsets supported by the
converter (the most common latin, japanese, chinese, korean and cyrillic code
pages are supported), into UTF-8.

Any error attempt to decode the string will cause a lua error to propagate.

The return value, on successful decoding, will be a representation of
`BINARY_INPUT` encoded as UTF-8.
