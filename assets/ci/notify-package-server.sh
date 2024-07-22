#!/bin/bash
# Requires; TOKEN=package rebuild token

t=$(mktemp)
trap "rm -f $t" EXIT
printenv TOKEN > $t

curl -X POST -v https://pkgs.kumomta.com/api/rebuild \
  -H "Content-Type: application/json" \
  -d "{\"token\": \"$(< $t)\"}"

