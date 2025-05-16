# filesizeformat

```rust
pub fn filesizeformat(value: f64, binary: Option<bool>) -> String
```

Formats the value like a “human-readable” file size.

For example. 13 kB, 4.1 MB, 102 Bytes, etc. Per default decimal prefixes are
used (Mega, Giga, etc.), if the second parameter is set to true the binary
prefixes are used (Mebi, Gibi).
