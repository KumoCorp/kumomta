# headers

Specify a list of message headers to include in the logs. The default is
empty.

```lua
kumo.configure_local_logs {
  -- ..
  headers = { 'Subject' },
}
```

{{since('2023.12.28-63cde9c7', indent=True)}}
    Header names can now use simple wildcard suffixes; if the last character
    of the header name is `*` then it will match any string with that prefix.
    For example `"X-*"` will match any header names that start with `"X-"`.


