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
	./assets/run-lua-test

test: build test-lua
	./docs/update-openapi.sh
	cargo nextest run --no-fail-fast

test-kumod:
	cargo nextest run --no-fail-fast -p kumod

clippy:
	cargo clippy -- \
		-A clippy::assertions_on_constants \
		-A clippy::upper_case_acronyms \
		-A clippy::collapsible_if \
		-A clippy::comparison_chain \
		-A clippy::drop_non_drop \
		-A clippy::if_same_then_else \
		-A clippy::inherent_to_string \
		-A clippy::int_plus_one \
		-A clippy::len_without_is_empty \
		-A clippy::manual_c_str_literals \
		-A clippy::manual_flatten \
		-A clippy::manual_strip \
		-A clippy::match_like_matches_macro \
		-A clippy::multiple_bound_locations \
		-A clippy::module_inception \
		-A clippy::needless_bool \
		-A clippy::needless_borrow \
		-A clippy::needless_lifetimes \
		-A clippy::needless_option_as_deref \
		-A clippy::needless_range_loop \
		-A clippy::needless_return \
		-A clippy::option_map_unit_fn \
		-A clippy::redundant_closure \
		-A clippy::redundant_guards \
		-A clippy::self_named_constructors \
		-A clippy::single_match \
		-A clippy::to_string_trait_impl \
		-A clippy::too_many_arguments \
		-A clippy::type_complexity \
		-A clippy::unnecessary_map_or \
		-A clippy::unnecessary_mut_passed \
		-A clippy::unnecessary_to_owned \
		-A clippy::useless_format \
		-A clippy::while_let_on_iterator \
		-A clippy::wrong_self_convention \

fmt:
	cargo +nightly fmt
	stylua --config-path stylua.toml .
	black docs/generate-toc.py assets/ci/build-builder-images.py assets/ci/emit-builder-dockerfile.py assets/bt

sink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026
	sudo iptables -t nat -L -n
	./target/release/kumod --user `id -un` --policy sink.lua
	#smtp-sink 127.0.0.1:2026 2000 || exit 0

smartsink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026
	sudo iptables -t nat -L -n
	SINK_PORT=2026 SINK_HTTP=8002 SINK_SPOOL=/tmp/kumo-sink SINK_DATA=`pwd`/examples/smart-sink-docker/policy/responses.toml ./target/release/kumod --user `id -un` --policy `pwd`/examples/smart-sink-docker/policy/init.lua

hugesink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 192.168.1.54:2026
	sudo iptables -t nat -L -n
	#smtp-sink 127.0.0.1:2026 2000 || exit 0

unsink: # float?
	while sudo iptables -t nat -D OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026 ; do true ; done
