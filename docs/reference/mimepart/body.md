# body

```lua
local content = mimepart.body
mimepart.body = 'changed content'
```

!!! note
    This is a field rather than a method, so you must use `mimepart.body`
    rather than `mimepart:body` to reference it.

{{since('dev')}}

The `body` field allows reading or writing the transfer-decoded content of the
`mimepart`.  For example, if the incoming message has `base64` encoded the
content and applied a `Content-Transfer-Encoding` header on the part to
indicate that it is base64 encoded, `mimepart.body` will base64-decode the
content before returning the bytes to your code.

If/when you assign the `body` field, appropriate transfer encoding will be
applied to the raw content that you provide.

!!! note
    Replacing the content doesn't implicitly change the `Content-Type` of the
    part, so you are responsible for ensuring that any modification you make to
    the part keeps the resulting message logically correct.  See also
    [mimepart:replace_body](replace_body.md) for a variation of this that
    allows you to change the `Content-Type` as part of the assignment.
