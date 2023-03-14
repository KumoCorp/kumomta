#!/bin/bash
set -ex
PREFIX="${1:-/opt/kumomta}"

mkdir -p ${PREFIX}/sbin ${PREFIX}/share/bounce_classifier
install -Dsm755 target/release/kumod -t ${PREFIX}/sbin
install -Dsm755 target/release/traffic-gen -t ${PREFIX}/sbin
install -Dm644 assets/bounce_classifier/* -t ${PREFIX}/share/bounce_classifier

