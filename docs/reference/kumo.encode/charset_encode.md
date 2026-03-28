# kumo.encode.charset_encode

```lua
kumo.encode.charset_encode(CHARSET, UTF8_INPUT, OPT_IGNORE_ERRORS)
```

{{since('dev')}}

Given an the input string `UTF8_INPUT`, which must be a UTF8 string, encodes it
into the named `CHARSET`, which must be one of the charsets supported by the
converter (the most common latin, japanese, chinese, korean and cyrillic code
pages are supported).

If `OPT_IGNORE_ERRORS` is set to `false`, any errors representing the input
string as the requested charset will cause a lua error to be propagated.
Setting `OPT_IGNORE_ERRORS` to `true`, or omitting `OPT_IGNORE_ERRORS`, will
cause the result to be potentially a partial encoding of the input.

The return value will be a byte string which, barring any encoding errors, will
be the representation of the input string in the requested charset.
