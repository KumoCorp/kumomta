# kumomta

## Debugging/Tracing

This will launch the server using the policy defined in [simple_policy.lua](simple_policy.lua):

```
KUMOD_LOG=kumod=trace cargo run -p kumod -- --policy simple_policy.lua
```

## Contributing

Ensure that the code is formatted before submitting a PR.

You need to install [StyLua](https://github.com/JohnnyMorganz/StyLua) to
format lua:

```bash
$ cargo install stylua --features lua54
```

Then you can format both the rust and the lua code:

```bash
$ make fmt
```

### Fuzzing

Some components have fuzzer coverage.  To run it, follow the setup from [the
Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz/setup.html)

Then:

```bash
$ cd crates/rfc5321/
$ cargo +nightly fuzz run parser
```
