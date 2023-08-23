maildir
===
![Build Status](https://github.com/staktrace/maildir/actions/workflows/test.yml/badge.svg)
[![Crate](https://img.shields.io/crates/v/maildir.svg)](https://crates.io/crates/maildir)

A simple library to deal with maildir folders

API
---
The primary entry point for this library is the Maildir structure, which can be created from a path, like so:

```rust
    let maildir = Maildir::from("path/to/maildir");
```

The Maildir structure then has functions that can be used to access and modify mail files.

Documentation
---
See the rustdoc at [docs.rs](https://docs.rs/maildir/).

Support maildir
---
If you want to support development of `maildir`, please do so by donating your money, time, and/or energy to fighting climate change.
A quick and easy way is to send a donation to [Replant.ca Environmental](http://www.replant-environmental.ca/donate.html), where every dollar gets a tree planted!
