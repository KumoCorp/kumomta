#!/bin/sh
set -xe

KUMO_POLICY="${KUMO_POLICY:-/config/policy.lua}"

exec /app/kumod --policy "${KUMO_POLICY}"


