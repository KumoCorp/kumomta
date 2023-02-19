#!/bin/bash
set -ex

# compile a static binary using musl libc
sudo docker run --rm -it -v "$(pwd)":/home/rust/src messense/rust-musl-cross:x86_64-musl cargo build --release

# Build image
sudo docker build -t kumomta/smtpd .
