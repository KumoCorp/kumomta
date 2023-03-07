#!/bin/bash

tracked_markdown=$(mktemp)
trap "rm ${tracked_markdown}" "EXIT"
git ls-tree -r HEAD --name-only docs | egrep '\.(markdown|md)$' > $tracked_markdown

for doc_dep in gelatyx mdbook mdbook-linkcheck mdbook-mermaid mdbook.admonish ; do
  if ! hash $doc_dep 2>/dev/null ; then
    cargo install $doc_dep --locked
  fi
done

gelatyx --language lua --file-list $tracked_markdown --language-config stylua.toml
gelatyx --language lua --file-list $tracked_markdown --language-config stylua.toml --check || exit 1

set -x

python3 docs/generate-toc.py || exit 1

mdbook-mermaid install docs
mdbook build docs

