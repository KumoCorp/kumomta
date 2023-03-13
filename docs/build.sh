#!/bin/bash
set -e
set -x

tracked_markdown=$(mktemp)
trap "rm ${tracked_markdown}" "EXIT"
find docs -type f | egrep '\.(markdown|md)$' > $tracked_markdown

mode=mkdocs

# We use mdbook-linkcheck even with mkdocs because we could not find any
# good mkdocs link checking tools that work properly with local links
cargo_deps="gelatyx mdbook-linkcheck"

if [ $mode == "mdbook" ]; then
  cargo_deps="$cargo_deps mdbook mdbook-mermaid mdbook-admonish"
fi
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

python3 docs/generate-toc.py $mode || exit 1

case $mode in
  mkdocs)
    # Run mdbook-linkcheck and clean up its junk
    mdbook-linkcheck --standalone docs

    # Adjust path to pick up pip-installed binaries
    PATH="$HOME/.local/bin;$PATH"
    pip install --quiet mkdocs-material pillow cairosvg mkdocs-git-revision-date-localized-plugin black mkdocs-exclude
    mkdocs build
    # Keep the toc generator formatted
    black docs/generate-toc.py
    ;;
  mdbook)
    mdbook-mermaid install docs
    mdbook build docs
    ;;
esac

