#!/bin/bash
set -xe
MAIN_TARGET="/build-cache/cargo-target_${GROUP}"

PATH=$PATH:$HOME/.cargo/bin
export CARGO_TARGET_DIR="${MAIN_TARGET}"
export CARGO_HOME="/build-cache/cargo-home"

# Workaround lack of --no-dereference in xcp.
# The criteria is really "if source dir has symlinks-to-dirs",
# but we compromise and approximate as "if source dir has symlinks to other than .so"
# as a lower effort test to write here
# <https://github.com/tarka/xcp/issues/52>
function can_use_xcp() {
  if hash xcp 2>/dev/null ; then
    if find "$1" -type l | grep -q -v \.so ; then
      # echo "Have symlinks"
      return 1
    fi
    # echo "no symlinks"
    return 0
  else
    # echo "no xcp"
    return 0
  fi
}

case "${CI_PIPELINE_EVENT}" in
  pull_request*)
    # Copy the mainline cache and use that as a basis
    CARGO_TARGET_DIR="${CI_WORKSPACE}/target"
    if test -d "${MAIN_TARGET}" ; then
      if can_use_xcp "${MAIN_TARGET}" ; then
        time xcp --recursive --no-progress --workers=0 "${MAIN_TARGET}" "${CARGO_TARGET_DIR}"
      else
        time cp -p --recursive --no-dereference "${MAIN_TARGET}" "${CARGO_TARGET_DIR}"
      fi
    fi
  ;;
esac

cat >.cache-env <<-EOT
export CARGO_INCREMENTAL=0
export CARGO_HOME="${CARGO_HOME}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR}"
export PATH="${PATH}"
EOT
