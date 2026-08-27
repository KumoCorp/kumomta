# pick up env vars from an optional .make-env file.
# On ubuntu you probably want ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu
# in .make-env to make it faster to build the rocksdb crate
-include .make-env
export $(shell test -f .make-env && sed 's/=.*//' .make-env)

check:
	cargo check

build:
	cargo build $(BUILD_OPTS) -p kumod
	cargo build $(BUILD_OPTS) -p tsa-daemon
	cargo build $(BUILD_OPTS) -p kcli
	cargo build $(BUILD_OPTS) -p validate-shaping
	cargo build $(BUILD_OPTS) -p proxy-server
	cargo build $(BUILD_OPTS) -p spool-util
	cargo build $(BUILD_OPTS) -p tailer
	cargo build $(BUILD_OPTS) -p traffic-gen
	cargo build $(BUILD_OPTS) -p toml2jsonc
	cargo build $(BUILD_OPTS) -p tls-probe

# Check compilation with all possible feature combinations
# Requires: cargo install --locked cargo-feature-combinations
fc:
	RUSTFLAGS="--cfg tokio_unstable -D warnings" cargo fc check --fail-fast

test-lua:
	cargo run -p run-lua-tests

test: build test-lua
	./docs/update-openapi.sh
	RUST_BACKTRACE=1 cargo nextest run --no-fail-fast

int-test: build
	RUST_BACKTRACE=1 cargo nextest run --no-fail-fast

test-adhoc: build
	cargo nextest run --no-fail-fast --no-capture -p integration-tests -- mx_list_refresh

test-kumod:
	cargo nextest run --no-fail-fast -p kumod

expand-kumod:
	cargo expand -p kumod metrics_helper

macro-kumod:
	RUSTFLAGS="-Z macro-backtrace --cfg tokio_unstable" cargo +nightly check -p kumod

clippy:
	cargo clippy

fmt:
	cargo +nightly fmt
	cd crates/mod-smtp-response-normalize/codegen && cargo +nightly fmt
	stylua --config-path stylua.toml .
	black docs/generate-toc.py docs/update-metrics.py assets/ci/build-builder-images.py assets/ci/emit-builder-dockerfile.py assets/bt assets/log-filter.py

sink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026
	sudo iptables -t nat -L -n
	KUMO_NODE_ID=906fd326-34e6-4405-a086-971017bf0f10 ./target/release/kumod --user `id -un` --policy sink.lua
	#smtp-sink 127.0.0.1:2026 2000 || exit 0

smartsink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026
	sudo iptables -t nat -L -n
	KUMO_NODE_ID=053227a3-8663-4f4e-97f4-a91e9bcd022b SINK_PORT=2026 SINK_HTTP=8002 SINK_SPOOL=/tmp/kumo-sink SINK_DATA=`pwd`/examples/smart-sink-docker/policy/responses.toml ./target/release/kumod --user `id -un` --policy `pwd`/examples/smart-sink-docker/policy/init.lua

hugesink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 192.168.1.54:2026
	sudo iptables -t nat -L -n
	#smtp-sink 127.0.0.1:2026 2000 || exit 0

unsink: # float?
	while sudo iptables -t nat -D OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026 ; do true ; done
