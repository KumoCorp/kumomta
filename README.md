# kumomta

## Debugging/Tracing

```
KUMOD_LOG=kumod=trace cargo run -p kumod
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
