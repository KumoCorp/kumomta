#!/bin/sh
# This script updates the snapshot of the openapi specs for our
# various services, so that the mkdocs build can consume them
# to render the docs in the docs.
# It needs to be run manually after changing anything to do with the
# HTTP APIs.
./target/debug/kumod --dump-openapi-spec > docs/reference/kumod.openapi.json
./target/debug/tsa-daemon --dump-openapi-spec > docs/reference/tsa-daemon.openapi.json
