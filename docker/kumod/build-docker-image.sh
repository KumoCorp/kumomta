#!/bin/bash
set -ex

# compile a static binary using musl libc
sudo docker run --rm -v "$(pwd)":/home/rust/src messense/rust-musl-cross:x86_64-musl cargo build --release

# Copy that binary into the docker build context
cp target/x86_64-unknown-linux-musl/release/kumod docker/smtpd/
trap "rm docker/smtpd/kumod" EXIT

# Build image
sudo docker build -t kumomta/smtpd ./docker/smtpd
