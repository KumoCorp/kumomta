#!/bin/bash
set -xe
MAIN_TARGET="/build-cache/cargo-target_${GROUP}"

PATH=$PATH:$HOME/.cargo/bin
export CARGO_TARGET_DIR="${MAIN_TARGET}"
export CARGO_HOME="/build-cache/cargo-home"

case "${CI_PIPELINE_EVENT}" in
  pull_request*)
    # Copy the mainline cache and use that as a basis
    CARGO_TARGET_DIR="${CI_WORKSPACE}/target"
    if test -d "${MAIN_TARGET}" ; then
      if hash xcp 2>/dev/null ; then
        time xcp --recursive --no-progress --workers=0 "${MAIN_TARGET}" "${CARGO_TARGET_DIR}"
      else
        time cp -p --recursive "${MAIN_TARGET}" "${CARGO_TARGET_DIR}"
      fi
    fi
  ;;
esac

cat >.cache-env <<-EOT
CARGO_HOME="${CARGO_HOME}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR}"
PATH="${PATH}"
EOT
