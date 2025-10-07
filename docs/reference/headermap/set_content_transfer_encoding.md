# set_content_transfer_encoding

```lua
headers:set_content_transfer_encoding(VALUE)
```

{{since('2025.10.06-5ec871ab')}}

Assign the `VALUE` to the `Content-Transfer-Encoding` header.

`VALUE` may be either a `string` or be an [MimeParams](index.md#mimeparams).

If you assign using a string, the string will be parsed and validated as being
compatible with [MimeParams](index.md#mimeparams) before allowing the assigment to proceed.

!!! danger
    Changing the `Content-Transfer-Encoding` header may result in an inconsistent
    representation of the message and should be avoided.  We recommend limiting
    changes to just the parameter portion of the header rather than the overall
    encoding.
