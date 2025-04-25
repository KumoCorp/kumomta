---
tags:
 - logging
---

# compression_level

Specifies the level of *zstd* compression that should be used.  Compression
cannot be disabled.

Specifying `0` uses the zstd default compression level, which is `3` at the
time of writing.

Possible values are `1` (cheapest, lightest) through to `21`.

```lua
kumo.configure_local_logs {
  -- ..
  compression_level = 3,
}
```


