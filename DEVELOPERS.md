# kumomta

## Debugging/Tracing

This will launch the server using the policy defined in [simple_policy.lua](simple_policy.lua):

```
KUMOD_LOG=kumod=trace cargo run -p kumod -- --policy simple_policy.lua
```

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

To contribute to this project, fork it, make your edits, test and then submit a PR from the fork.

Please make one change request per PR to make it easier to approve.

Document your PR clearly with an explanation of your reasons and the changes requested.

If you include or link to any 3rd party code, fully document the source and the reason.

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

If you are making documentation edits, you should also make sure Black is installed. One of these should work for you:

```console
pip install black
sudo apt install black
```
Now you can edit files under `~\kumomta\docs\`

Remember to update the Navigation menus in `generate-toc.py` if pages were added or deleted.

Build the new docs if needed with `docs/build.sh` from ~/kumomta/



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
