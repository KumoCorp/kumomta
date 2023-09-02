#!/bin/bash
# Sync the vendored sources from a URL
# eg:
# wget 'https://nlnetlabs.nl/downloads/unbound/unbound-1.18.0.tar.gz'
# import-unbound.sh path/to/unbound-1.18.0.tar.gz
set -x
TARBALL=$1

rm -rf unbound
tar xf $TARBALL
mv unbound-* unbound
rm -rf unbound/{testdata,pythonmod,testcode,winrc,contrib,configure,aclocal.m4,ltmain.sh,install-sh,config.guess,smallapp,doc,.git*,*.m4}

echo > unbound/dnscrypt/dnscrypt_config.h
echo > unbound/dnstap/dnstap_config.h
