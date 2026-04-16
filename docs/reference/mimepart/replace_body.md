# replace_body

```lua
mimepart:replace_body(CONTENT, OPTIONAL_CONTENT_TYPE)
```

{{since('2025.10.06-5ec871ab')}}

This method allows you to change the body portion of the `mimepart`, optionally
changing the `Content-Type` in the process.

`CONTENT` must be a lua string (either UTF-8 or binary) representing the actual
content you want to assign; `mimepart:replace_body` will select and apply
appropriate transfer encoding to the mime part.

`OPTIONAL_CONTENT_TYPE` is an optional string specifying the content type. If
omitted, if the part already has a content type, that content type will be
preserved. Otherwise, if there is no content type, a default content type will
be selected based on the binary or text nature of `CONTENT`.

The example below unilaterally changes the body of a mime part to plain text:

```lua
mimepart:replace_body('changed content', 'text/plain')
```

See also:

  * [mimepart.body](body.md)

