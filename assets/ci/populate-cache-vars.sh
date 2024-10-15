#!/bin/bash
set -xe
PATH=$PATH:$HOME/.cargo/bin

# export SCCACHE_C_CUSTOM_CACHE_BUSTER="${GROUP}-$(uname -m)-1"
cat >.cache-env <<-EOT
export RUSTC_WRAPPER=$HOME/.cargo/bin/sccache
export SCCACHE_DIR=/build-cache/sccache
export SCCACHE_DIRECT=true
export SCCACHE_CACHE_SIZE="100G"
export CARGO_INCREMENTAL=0
export PATH="${PATH}"
EOT

# Now look for rocksdb stuff that we may have pre-built in the image
# and update the environment so that we'll use that in the build
ROCKSDB_LIB_DIR=$(dirname /opt/kumomta/lib*/librocksdb.a)
if test -d "${ROCKSDB_LIB_DIR}" ; then
  echo "export ROCKSDB_LIB_DIR=${ROCKSDB_LIB_DIR}" >> .cache-env
  echo "export ROCKSDB_STATIC=static" >> .cache-env
fi

SNAPPY_LIB_DIR=$(dirname /opt/kumomta/lib*/libsnappy.a)
if test -d "${SNAPPY_LIB_DIR}" ; then
  echo "export SNAPPY_LIB_DIR=${SNAPPY_LIB_DIR}" >> .cache-env
  echo "export SNAPPY_STATIC=static" >> .cache-env
fi

cat .cache-env

