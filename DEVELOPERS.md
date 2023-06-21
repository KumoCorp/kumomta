# kumomta

## Debugging/Tracing

This will launch the server using the policy defined in [simple_policy.lua](simple_policy.lua):

```
KUMOD_LOG=kumod=trace cargo run -p kumod -- --policy simple_policy.lua
```

## Tokio Console

You may use
[tokio-console](https://docs.rs/tokio-console/latest/tokio_console/) to
introspect the system. You must first start the server with tokio tracing
enabled:

```
KUMOD_LOG=tokio=trace,runtime=trace,info cargo run -p kumod -- --tokio-console --policy simple_policy.lua 2>/dev/null
```

Note the `2>/dev/null`; that is to prevent tracing log spew from hitting the terminal.

Then, in another window, run `tokio-console`.

The following environment variables influence the tokio console server that is
embedded into kumod:

| **Environment Variable**         | **Purpose**                                                  | **Default Value** |
|----------------------------------|--------------------------------------------------------------|-------------------|
| `TOKIO_CONSOLE_RETENTION`        | The duration of seconds to accumulate completed tracing data | 3600s (1h)        |
| `TOKIO_CONSOLE_BIND`             | a HOST:PORT description, such as `localhost:1234`            | `127.0.0.1:6669`  |
| `TOKIO_CONSOLE_PUBLISH_INTERVAL` | The duration to wait between sending updates to the console  | 1000ms (1s)       |
| `TOKIO_CONSOLE_RECORD_PATH`      | The file path to save a recording                            | None              |

## Metrics

If the http listener is enabled, the `/metrics` endpoint will return a set of metrics
for prometheus to scrape.

You can manually review them with curl:

```
$ curl 'http://127.0.0.1:8000/metrics'
# HELP connection_count connection_count
# TYPE connection_count gauge
connection_count{service="esmtp_listener"} 1
connection_count{service="smtp_client:(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com."} 0
# HELP delayed_count delayed_count
# TYPE delayed_count gauge
delayed_count{queue="gmail.com"} 1
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
$ rustup toolchain install nightly
$ make fmt
```

### Docker build

To build a lightweight alpine-based docker image:

```
$ ./docker/kumod/build-docker-image.sh
...
$ docker image ls kumomta/kumod
REPOSITORY      TAG       IMAGE ID       CREATED         SIZE
kumomta/kumod   latest    bbced15ff4d1   3 minutes ago   116MB
```

You can then run that image; this invocation mounts the kumo
src dir at `/config` and then the `KUMO_POLICY` environment
variable is used to override the default `/config/policy.lua`
path to use the SMTP sink policy script [sink.lua](sink.lua),
which will accept and discard all mail:

```
$ sudo docker run --rm -p 2025:25 \
    -v .:/config \
    --name kumo-sink \
    --env KUMO_POLICY="/config/sink.lua" \
    kumomta/kumod
```

### Fuzzing

Some components have fuzzer coverage.  To run it, follow the setup from [the
Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz/setup.html)

Then:

```bash
$ cd crates/rfc5321/
$ cargo +nightly fuzz run parser
```
