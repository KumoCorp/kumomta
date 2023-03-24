#!/bin/bash
set -ex
PREFIX="${1:-/opt/kumomta}"

mkdir -p ${PREFIX}/sbin ${PREFIX}/share/bounce_classifier
install -Dsm755 target/release/kumod -t ${PREFIX}/sbin
install -Dsm755 target/release/traffic-gen -t ${PREFIX}/sbin
install -Dm644 assets/bounce_classifier/* -t ${PREFIX}/share/bounce_classifier

if test "$EUID" -eq 0 && getent passwd kumod >/dev/null && getent group kumod >/dev/null ; then
  for dir in /opt/kumomta/etc /opt/kumomta/etc/policy ; do
    [ -d "$dir" ] || install -d --mode 755 --owner kumod --group kumod $dir
  done

  for dir in /var/spool/kumomta /var/log/kumomta /opt/kumomta/etc/dkim ; do
    [ -d "$dir" ] || install -d --mode 2770 --owner kumod --group kumod $dir
  done
fi
