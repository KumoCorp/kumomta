# meta

Specify a list of message meta fields to include in the logs. The default is
empty.

```lua
kumo.configure_local_logs {
  -- ..
  meta = { 'my-meta-1' },
}
```


