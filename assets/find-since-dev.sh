#!/bin/bash
tag=$1
if [[ -z "$tag" ]] ; then
  echo "Usage: ./assets/find-since-dev.sh 2026.03.04-bb93ecb1"
  exit 1
else
  perl -pi -w -e "s/since\('dev'/since('$tag'/g;" $(rg -l "since\('dev'" docs crates)
fi
