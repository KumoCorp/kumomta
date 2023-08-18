#!/bin/bash
# This script only exists because GH Actions is pretty tight on disk space
# It's purpose is to reduce the target dir down to just the build artifacts

rm -rf ./target/{release,debug}/{example,build,deps}

