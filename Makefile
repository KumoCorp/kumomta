check:
	cargo check

build:
	cargo build $(BUILD_OPTS) -p kumod
	cargo build $(BUILD_OPTS) -p tsa-daemon
	cargo build $(BUILD_OPTS) -p kcli
	cargo build $(BUILD_OPTS) -p validate-shaping
	cargo build $(BUILD_OPTS) -p proxy-server
	cargo build $(BUILD_OPTS) -p tailer
	cargo build $(BUILD_OPTS) -p traffic-gen
	cargo build $(BUILD_OPTS) -p toml2jsonc

# Check compilation with all possible feature combinations
# Requires: cargo install --locked cargo-feature-combinations
fc:
	RUSTFLAGS="--cfg tokio_unstable -D warnings" cargo fc check --fail-fast

test-lua:
	./assets/run-lua-test

test: build test-lua
	./docs/update-openapi.sh
	cargo nextest run --no-fail-fast

fmt:
	cargo +nightly fmt
	stylua --config-path stylua.toml .
	black docs/generate-toc.py assets/ci/build-builder-images.py assets/ci/emit-builder-dockerfile.py assets/bt

sink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026
	sudo iptables -t nat -L -n
	./target/release/kumod --user `id -un` --policy sink.lua
	#smtp-sink 127.0.0.1:2026 2000 || exit 0

hugesink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 192.168.1.54:2026
	sudo iptables -t nat -L -n
	#smtp-sink 127.0.0.1:2026 2000 || exit 0

unsink: # float?
	while sudo iptables -t nat -D OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026 ; do true ; done
