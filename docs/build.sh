#!/bin/bash

tracked_markdown=$(mktemp)
trap "rm ${tracked_markdown}" "EXIT"
git ls-tree -r HEAD --name-only docs | egrep '\.(markdown|md)$' > $tracked_markdown

if ! hash gelatyx 2>/dev/null ; then
  cargo install gelatyx --locked
fi

gelatyx --language lua --file-list $tracked_markdown --language-config stylua.toml
gelatyx --language lua --file-list $tracked_markdown --language-config stylua.toml --check || exit 1

set -x

python3 docs/generate-toc.py || exit 1

if ! hash mdbook 2>/dev/null ; then
  cargo install mdbook --locked
fi
if ! hash mdbook-linkcheck 2>/dev/null ; then
  cargo install mdbook-linkcheck --locked
fi
if ! hash mdbook-mermaid 2>/dev/null ; then
  cargo install mdbook-mermaid --locked
fi

mdbook-mermaid install docs
mdbook build docs

