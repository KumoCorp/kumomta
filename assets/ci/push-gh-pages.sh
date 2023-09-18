#!/bin/bash
set -e

cd gh_pages
git init
git remote add origin https://x-access-token:${TOKEN}@github.com/KumoCorp/kumomta
echo docs.kumomta.com > CNAME
git add --all
git commit -am "Deploy docs $(date)"

git push -f origin HEAD:gh-pages
