#!/bin/bash
# This script is an implementation detail of the package builder automatioon.
# It is not intended to be run directly by humans.
# It may change its behavior in unpredictable ways and should not be relied
# upon by anyone or anything other than build-deb.sh and build-rpm.sh
set -ex
PREFIX="${1:-/opt/kumomta}"
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-${PWD}/target}
# If KEEP_DEBUG==yes, we preserve debug info for kumod and tsa-daemon only.
# The other binaries we always strip to balance overall package size.
STRIP=
[[ "${KEEP_DEBUG}" == "yes" ]] || STRIP="-s"

mkdir -p ${PREFIX}/sbin ${PREFIX}/share/bounce_classifier ${PREFIX}/share/policy-extras ${PREFIX}/share/community
install -Dsm755 ${CARGO_TARGET_DIR}/${TRIPLE}release/validate-shaping -t ${PREFIX}/sbin
install -Dm755  ${STRIP} ${CARGO_TARGET_DIR}/${TRIPLE}release/tsa-daemon -t ${PREFIX}/sbin
install -Dsm755 ${CARGO_TARGET_DIR}/${TRIPLE}release/proxy-server -t ${PREFIX}/sbin
install -Dm755  ${STRIP} ${CARGO_TARGET_DIR}/${TRIPLE}release/kumod -t ${PREFIX}/sbin
install -Dsm755 ${CARGO_TARGET_DIR}/${TRIPLE}release/kcli -t ${PREFIX}/sbin
install -Dsm755 ${CARGO_TARGET_DIR}/${TRIPLE}release/traffic-gen -t ${PREFIX}/sbin
install -Dsm755 ${CARGO_TARGET_DIR}/${TRIPLE}release/tailer -t ${PREFIX}/sbin
install -Dsm755 ${CARGO_TARGET_DIR}/${TRIPLE}release/toml2jsonc -t ${PREFIX}/sbin
install -Dsm755 ${CARGO_TARGET_DIR}/${TRIPLE}release/tls-probe -t ${PREFIX}/sbin
install -Dm755 assets/accounting.sh -t ${PREFIX}/sbin
install -Dm755 assets/explain-throttle -t ${PREFIX}/sbin
install -Dm755 assets/resolve-site-name -t ${PREFIX}/sbin
install -Dm755 assets/resolve-queue-config -t ${PREFIX}/sbin
install -Dm755 assets/resolve-shaping-domain -t ${PREFIX}/sbin
install -Dm644 assets/bounce_classifier/* -t ${PREFIX}/share/bounce_classifier
install -Dm644 assets/init.lua -T ${PREFIX}/share/minimal-init.lua
install -Dm644 assets/tsa_init.lua -T ${PREFIX}/share/minimal-tsa_init.lua
install -Dm644 assets/policy-extras/*.lua -t ${PREFIX}/share/policy-extras
install -Dm644 assets/policy-extras/*.toml -t ${PREFIX}/share/policy-extras
install -Dm644 assets/community/*.toml -t ${PREFIX}/share/community

if test "$EUID" -eq 0 && getent passwd kumod >/dev/null && getent group kumod >/dev/null ; then
  for dir in /opt/kumomta/etc /opt/kumomta/etc/policy ; do
    [ -d "$dir" ] || install -d --mode 755 --owner kumod --group kumod $dir
  done

  for dir in /var/spool/kumomta /var/log/kumomta /opt/kumomta/etc/dkim ; do
    [ -d "$dir" ] || install -d --mode 2770 --owner kumod --group kumod $dir
  done
fi
