#!/bin/bash
set -ex
top_dir="$(git rev-parse --show-toplevel)"
cd "${top_dir}"
docker build -t kumomta/kumod --build-context "src=${top_dir}" ./docker/kumod
