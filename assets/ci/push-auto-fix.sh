#!/bin/bash
set -e

if ! git diff-index --quiet HEAD -- ; then
  echo "There are pending changes:"
  git diff-index --name-status HEAD --
  AUTHOR=$(git show -s --format='%aN')
  EMAIL=$(git show -s --format='%aE')
  git config user.name "${AUTHOR}"
  git config user.email "${EMAIL}"
  git remote set-url origin https://x-access-token:${TOKEN}@github.com/KumoCorp/kumomta
  git commit -am "Automated formatting fix"
  git push
fi
