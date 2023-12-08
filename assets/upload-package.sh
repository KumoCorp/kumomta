#!/bin/bash
# Requires; TOKEN=openrepo API token populated from secrets
REPO=$1
shift
PACKAGES="$@"

case "$REPO" in
  "ubuntu:18.04") REPO="kumomta-ubuntu-18" ;;
  "ubuntu:20.04") REPO="kumomta-ubuntu-20" ;;
  "ubuntu:22.04") REPO="kumomta-ubuntu-22" ;;
  "arm64v8/ubuntu:22.04") REPO="kumomta-ubuntu-aarch64-22" ;;
  "rockylinux:8") REPO="kumomta-rockylinux-8" ;;
  "rockylinux:9") REPO="kumomta-rockylinux-9" ;;
  "centos:7") REPO="kumomta-centos-7" ;;
  "mcr.microsoft.com/cbl-mariner/base/core:2.0") REPO="kumomta-mariner-2" ;;
  "amazonlinux:2") REPO="kumomta-amazonlinux-2" ;;
  "amazonlinux:2023") REPO="kumomta-amazonlinux-2023" ;;
esac

[[ "${REF_TYPE}" == "tag" || "${CI_PIPELINE_EVENT}" == "tag" ]] && REPO="${REPO}-stable"

t=$(mktemp)
trap "rm -f $t" EXIT
printenv TOKEN > $t

for pkg in $PACKAGES ; do
  echo "Uploading $pkg"
  curl -X POST --silent https://openrepo.kumomta.com/api/$REPO/upload/ \
      -H "Authorization: Token $(< $t)" \
      -F "overwrite=1" \
      -F "package_file=@$pkg" -i
done
