#!/bin/bash
set -x
name=$(git -c "core.abbrev=8" show -s "--format=%cd-%h" "--date=format:%Y.%m.%d")

notes=$(cat <<EOT
See https://docs.kumomta.com/changelog/$name for the full changelog.
EOT
)

gh release view "$name" || gh release create --prerelease --notes "$notes" --title "$name" "$name"

PACKAGES="$@"
for pkg in $PACKAGES ; do
  echo "Uploading $pkg"
  bash ./assets/ci/retry.sh gh release upload --clobber "$name" $pkg
done
