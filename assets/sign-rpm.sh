#!/bin/bash
# Requires; PUB and PRIV env set to GPG public and private keys, populated
# with from the GH secrets for the org/repo
set -e

PACKAGES="$@"

t=$(mktemp)
trap "rm -f $t" EXIT

printenv PUB > $t
gpg --batch --import $t
printenv PRIV > $t
gpg --batch --import $t

# the `echo | setsid` trick is from:
# https://stackoverflow.com/a/57953409/149111
# Without this, signing prompts for input even though there is no password,
# which fails in the CI
echo "" | setsid rpmsign --define '_signature gpg' --define '_gpg_name KumoMTA Signing Key' --addsign $PACKAGES

echo "Signed $PACKAGES OK"
