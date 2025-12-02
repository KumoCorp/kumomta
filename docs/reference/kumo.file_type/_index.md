# kumo.file_type

{{since('2025.12.02-67ee9e96')}}

The `kumo.file_type` module includes functions that can help you to reason
about the file type of a sequence of bytes, or about the file types associated
with mime media types or filename extensions.

The underlying implementation of this module is provided by the
[file_type](https://docs.rs/file_type) Rust crate.

## FileTypeResult

The functions in this module all return a file type result which has the following shape:

```lua
local file_type = {
  -- The human readable name of the file type
  name = 'Java class file',
  -- The file type extensions, not including the dot
  extensions = { 'class' },
  -- The [RFC 2046 MIME Media Types](https://www.rfc-editor.org/rfc/rfc2046.html)
  -- associated with this file type
  media_types = {
    'application/java',
    'application/java-byte-code',
    'application/java-vm',
    'application/x-httpd-java',
    'application/x-java',
    'application/x-java-class',
    'application/x-java-vm',
  },
}
```

## Available Functions { data-search-exclude }


