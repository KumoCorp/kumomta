#!/bin/bash
ROCKS_VERSION=9.7.2
CMAKE_VERSION=3.30.5
SNAPPY_VERSION=1.2.1
set -xe -o pipefail

# This script builds a static version of rocksdb so that we can
# avoid recompiling it as often in the CI workflows.
# We cannot rely on the distro to provide rocksdb at all, let
# alone a consistent version of it, so we have to build it
# ourselves from a known-good version. Similarly, the spread
# of cmake and snappy versions on the target platforms is
# terrible, so we obtain and build specific versions of
# these in order to build successfully.

PREFIX=${PREFIX:-/tmp/rocks-build/installed}

mkdir -p "${PREFIX}" /tmp/rocks-build
cd /tmp/rocks-build

CMAKE_TAR=cmake-${CMAKE_VERSION}-linux-$(uname -m).tar.gz

if [ ! -f ${CMAKE_TAR} ] ; then
  curl -L  https://github.com/Kitware/CMake/releases/download/v${CMAKE_VERSION}/${CMAKE_TAR} > ${CMAKE_TAR}
fi

tar xzf ${CMAKE_TAR}

if [ ! -f rocksdb-${ROCKS_VERSION}.tar.gz ] ; then
  curl -L https://github.com/facebook/rocksdb/archive/refs/tags/v${ROCKS_VERSION}.tar.gz > rocksdb-${ROCKS_VERSION}.tar.gz
fi

if [ ! -f snappy-${SNAPPY_VERSION}.tar.gz ] ; then
  curl -L https://github.com/google/snappy/archive/${SNAPPY_VERSION}.tar.gz > snappy-${SNAPPY_VERSION}.tar.gz
fi

tar xzf snappy-${SNAPPY_VERSION}.tar.gz
cd snappy-${SNAPPY_VERSION}
mkdir build
cd build
# We force in -fPIE because otherwise the static libraries
# produced by cmake on some distros are not able to be linked
# into the resulting rust executable. This is not needed on
# every distro, so if you ever feel like removing those flags
# you must be sure to test on every supported distro first to
# make sure that you're not going to break anything!
../../cmake-${CMAKE_VERSION}-linux-$(uname -m)/bin/cmake .. \
  -D CMAKE_BUILD_TYPE=Release \
  -D CMAKE_CXX_FLAGS="-fPIE" \
  -D CMAKE_C_FLAGS="-fPIE" \
  -D CMAKE_INSTALL_PREFIX="${PREFIX}" \
  -D BUILD_SHARED_LIBS=OFF \
  -D BUILD_STATIC_LIBS=ON \
  -D SNAPPY_BUILD_BENCHMARKS=OFF \
  -D SNAPPY_BUILD_TESTS=OFF
make -j8 install || exit 1
cd ../..

tar xzf rocksdb-${ROCKS_VERSION}.tar.gz
cd rocksdb-${ROCKS_VERSION}
rm -rf build
mkdir build
cd build
../../cmake-${CMAKE_VERSION}-linux-$(uname -m)/bin/cmake .. \
  -D CMAKE_BUILD_TYPE=Release \
  -D CMAKE_INSTALL_PREFIX="${PREFIX}" \
  -D CMAKE_CXX_FLAGS="-fPIE" \
  -D CMAKE_C_FLAGS="-fPIE" \
  -D WITH_SNAPPY=ON \
  -D WITH_LZ4=OFF \
  -D WITH_TESTS=OFF \
  -D WITH_BENCHMARK_TOOLS=OFF \
  -D WITH_GFLAGS=OFF \
  -D FAIL_ON_WARNINGS=OFF \
  -D BUILD_SHARED_LIBS=OFF \
  -D BUILD_STATIC_LIBS=ON \
  -D ROCKSDB_BUILD_SHARED=OFF || exit 1
make -j8 install || exit 1
