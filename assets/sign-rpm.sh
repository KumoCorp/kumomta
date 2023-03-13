#!/bin/bash
# Requires; PUB and PRIV env set to GPG public and private keys, populated
# with from the GH secrets for the org/repo
PACKAGES="$@"

t=$(mktemp)
trap "rm -f $t" EXIT

printenv PUB > $t
gpg --batch --import $t
printenv PRIV > $t
gpg --batch --import $t

rpmsign --define '_signature gpg' --define '_gpg_name KumoMTA Signing Key' --addsign $PACKAGES

