# set_content_type

```lua
headers:set_content_type(VALUE)
```

{{since('dev')}}

Assign the `VALUE` to the `Content-Type` header.

`VALUE` may be either a `string` or be an [MimeParams](index.md#mimeparams).

If you assign using a string, the string will be parsed and validated as being
compatible with [MimeParams](index.md#mimeparams) before allowing the assigment to proceed.
