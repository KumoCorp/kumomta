#!/bin/sh
set -xe

KUMO_POLICY="${KUMO_POLICY:-/config/policy.lua}"

exec /opt/kumomta/sbin/kumod --policy "${KUMO_POLICY}" --user kumod


