#!/bin/bash
set -e
set -x

CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-${PWD}/target}

SERVE=no
if [ "$1" == "serve" ] ; then
  SERVE=yes
fi

tracked_markdown=$(mktemp)
trap "rm ${tracked_markdown}" "EXIT"
find docs -type f | egrep '\.(markdown|md)$' > $tracked_markdown

cargo_deps="gelatyx"

for doc_dep in $cargo_deps ; do
  if ! hash $doc_dep 2>/dev/null ; then
    cargo install $doc_dep --locked
  fi
done

if test -z "${CHECK_ONLY}" ; then
  gelatyx --language lua --file-list $tracked_markdown --language-config stylua.toml
fi
if ! gelatyx --language lua --color always --file-list $tracked_markdown --language-config stylua.toml --check ; then
  echo
  echo "Be sure to run ./docs/build.sh to apply formatting before you push changes."
  echo
  exit 1
fi

if test -x ${CARGO_TARGET_DIR}/debug/kumod ; then
  ./docs/update-openapi.sh
  ${CARGO_TARGET_DIR}/debug/kumod --dump-lruttl-caches > docs/reference/lruttl-caches.json
fi
if test -x ${CARGO_TARGET_DIR}/debug/kcli ; then
  ${CARGO_TARGET_DIR}/debug/kcli markdown-help
fi

# https://github.com/rust-lang/cargo/issues/2025
# Document only our own crates
# TODO: consider using:
# <https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#rustdoc-map>
cargo tree --depth 0 -e normal --prefix none | \
  cut -d' ' -f1 | sort -u | xargs printf -- '-p %s\n' | \
  xargs cargo doc --no-deps --locked --lib

if [ "$CI" == true ] ; then
  rm docs/rustapi
  ln -sf ${CARGO_TARGET_DIR}/doc docs/rustapi
fi

python3 docs/generate-toc.py || exit 1

# Adjust path to pick up pip-installed binaries
PATH="$HOME/.local/bin;$PATH"

# Keep the toc generator formatted
if hash black 2>/dev/null ; then
  black docs/generate-toc.py
fi

if [ "$CI" == true ] ; then
  exit 0
fi

docker_or_podman() {
  if hash podman 2>/dev/null ; then
    podman "$@"
  elif hash docker 2>/dev/null ; then
    docker "$@"
  else
    echo "Please install either podman or docker"
    exit 1
  fi
}

docker_or_podman build -t kumomta/mkdocs-material --pull -f docs/Dockerfile .
	
if [ "$SERVE" == "yes" ] ; then
  docker_or_podman run --rm -it --network=host -v ${PWD}:/docs kumomta/mkdocs-material $@
else
  docker_or_podman run --rm -e CARDS=${CARDS} -v ${PWD}:/docs kumomta/mkdocs-material build
fi
