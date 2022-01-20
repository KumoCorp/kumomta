# cfdkim

> DKIM ([RFC6376]) implementation

## Features

### Verifying email signatures

Example:
```rust
let res: DKIMResult = cfdkim::verify_email(&logger, &from_domain, &parsed_email).await?;

if let Some(err) = &res.error() {
  error!(logger, "dkim verify fail: {}", err);
}

println!("dkim={}", res.with_detail());
```

The `verify_email` arguments are the following:
- `logger`: [slog]::Logger
- `from_domain`: &str ([RFC5322].From's domain)
- `parsed_email`: [mailparse]::ParsedMail

### Signing an email

Work in progress.

[RFC5322]: https://datatracker.ietf.org/doc/html/rfc5322
[RFC6376]: https://datatracker.ietf.org/doc/html/rfc6376
[slog]: https://crates.io/crates/slog
[mailparse]: https://crates.io/crates/mailparse
