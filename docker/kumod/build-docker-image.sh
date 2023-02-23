#!/bin/bash
set -ex
top_dir="$(git rev-parse --show-toplevel)"
cd "${top_dir}"
docker build -t kumomta/kumod --file docker/kumod/Dockerfile .
