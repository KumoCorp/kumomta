#!/bin/sh
set -xe

KUMO_POLICY="${KUMO_POLICY:-/opt/kumomta/policy/init.lua}"

exec /opt/kumomta/sbin/kumod --policy "${KUMO_POLICY}" --user kumod


