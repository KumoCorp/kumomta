# Building KumoMTA from Source

If pre-built binaries are not provided for your system of choice, of if you'd
like try your hand at extending KumoMTA, you'll need to build it from source.

## Prepare your environment

Read the [Environmental
considerations](https://github.com/kumomta/kumomta/blob/main/docs/tutorial/environment_consideration.md)
before proceeding.  You will need a suitably sized server with all of the
prerequisites in order to be successful.

In addition, you will need to install some development packages.

## Obtain The Code

You will need `git`:

=== "RPM based systems"
    ```bash
    $ sudo dnf install -y git
    ```

=== "APT based systems"
    ```bash
    $ sudo apt install -y git
    ```

Then clone the repo:

```bash
$ git clone https://github.com/kumomta/kumomta.git
$ cd kumomta
```

## Install Dependencies

The `get-deps.sh` script in the repo knows how to install dependencies for
various systems; you should run it the first time you clone the repo,
and may need to run it after running a pull to update the repo in the future:

```bash
$ ./get-deps.sh
```

## Install Rust

You will need the Rust compiler to build KumoMTA.

We strongly recommend using [rustup](https://rustup.rs/) to install and manage
your Rust compiler. While some distributions offer a version of the Rust compiler,
it is often outdated.

If you are using a priviledged user, drop back to your non-priviledged user first:

```bash
$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
$ source ~/.profile
$ source ~/.cargo/env
$ rustc -V
```

## Building KumoMTA

With all the dependencies available, the actual build process is very simple:

```
$ cd kumomta
$ cargo build --release
```

This will build everything, leaving the binaries in the `target/release`
directory in the repo.

## Running from your source directory

This command will bring `kumod` up to date (in case you made changes), and then launch it:

```bash
$ KUMOD_LOG=kumod=info cargo run --release -p kumod -- --policy simple_policy.lua
```

In the above you are telling Cargo to run the Rust compiler to build an
optimized release version and package it as kumod, then execute kumod using the
policy file called `simple_policy.lua`.

You can add debugging output by adjusting the `KUMOD_LOG` environment variable.
For exampe, setting `KUMOD_LOG=kumod=trace` in the environment will run with
very verbose logging.

## Installing your build

```bash
$ install -m755 target/release/kumod -t /usr/bin
$ install -m755 target/release/traffic-gen -t /usr/bin
```

## Staying up to date

To synchronize your repo with the latest changes:

```bash
$ cd kumomta
$ git pull --rebase
$ cargo build --release
```

