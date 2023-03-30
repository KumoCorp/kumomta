#!/bin/bash
# This script creates a tag for a release, based on the current commit
TAG_NAME=${TAG_NAME:-$(git -c "core.abbrev=8" show -s "--format=%cd-%h" "--date=format:%Y.%m.%d")}
git tag $TAG_NAME
