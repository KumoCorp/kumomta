#!/bin/bash
# Requires; TOKEN=openrepo API token populated from secrets
REPO=$1
shift
PACKAGES="$@"

case "$REPO" in
  "ubuntu:18.04") REPO="kumomta-ubuntu-18" ;;
  "ubuntu:20.04") REPO="kumomta-ubuntu-20" ;;
  "ubuntu:22.04") REPO="kumomta-ubuntu-22" ;;
  "rockylinux:8") REPO="kumomta-rockylinux-8" ;;
  "rockylinux:9") REPO="kumomta-rockylinux-9" ;;
esac

t=$(mktemp)
trap "rm -f $t" EXIT
printenv TOKEN > $t

for pkg in $PACKAGES ; do
  curl -X POST https://openrepo.kumomta.com/api/$REPO/upload/ \
      -H "Authorization: Token $(< $t)" \
      -F "package_file=@$pkg" -i
done
