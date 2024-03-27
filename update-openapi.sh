#!/bin/sh
# This script updates the snapshot of the openapi specs for our
# various services, so that the mkdocs build can consume them
# to render the docs in the docs.
# It needs to be run manually after changing anything to do with the
# HTTP APIs.

# Only update the spec file if anything other than the version (which
# typically changes all the time during development, to track the current
# git hash) actually changed.
update_if_different() {
  binary=$1
  specfile=$2

  candidate=$(mktemp)
  trap "rm ${candidate}" "EXIT"
  current=$(mktemp)
  trap "rm ${current}" "EXIT"

  $binary --dump-openapi-spec | blank_out_openapi_spec_version > $candidate
  blank_out_openapi_spec_version < $specfile > $current
  if ! cmp $candidate $current ; then
    echo "$specfile updated"
    $binary --dump-openapi-spec > $specfile
  fi
}

# Replace info.version with "blank" in an openapi json doc
blank_out_openapi_spec_version() {
  jq --arg version 'blank' '.info.version = $version'
}

update_if_different ./target/debug/kumod docs/reference/kumod.openapi.json
update_if_different ./target/debug/tsa-daemon docs/reference/tsa-daemon.openapi.json
