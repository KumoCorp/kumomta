check:
	cargo check

test:
	cargo build
	./docs/update-openapi.sh
	cargo nextest run

fmt:
	cargo +nightly fmt
	stylua --config-path stylua.toml .
	black docs/generate-toc.py assets/ci/build-builder-images.py

sink: unsink
	sudo iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026
	sudo iptables -t nat -L -n
	smtp-sink 127.0.0.1:2026 2000 || exit 0

unsink: # float?
	while sudo iptables -t nat -D OUTPUT -p tcp \! -d 192.168.1.0/24 --dport 25 -j DNAT --to-destination 127.0.0.1:2026 ; do true ; done
