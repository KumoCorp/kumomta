# kumomta

## Concepts

### Queuing

Messages are assigned to a queue upon reception. The system can support
arbitrarily named queues but the convention is to construct the queue name from
some metadata associated with the message:

* `tenant` - some kind of sender or customer identifier or identity derived
  either from the message itself or authentication of the injection session.
* `campaign` - a sender-provided label that can be used to logically group a set
  of related messages, perhaps generated from the same campaign.
* destination domain - the site where the email will be routed

These three pieces of information are combined to produce the name of the queue
in the form `campaign:tenant@domain`.

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

### Docker build

To build a lightweight alpine-based docker image:

```
$ ./ci/build-docker-image.sh
...
$ sudo docker image ls kumomta/smtpd
REPOSITORY      TAG       IMAGE ID       CREATED         SIZE
kumomta/smtpd   latest    962d8f453c6b   8 minutes ago   22MB
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
    kumomta/smtpd
```

### Fuzzing

Some components have fuzzer coverage.  To run it, follow the setup from [the
Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz/setup.html)

Then:

```bash
$ cd crates/rfc5321/
$ cargo +nightly fuzz run parser
```
