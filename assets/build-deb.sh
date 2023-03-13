#!/bin/bash
set -x
set -e
TAG_NAME=${TAG_NAME:-$(git -c "core.abbrev=8" show -s "--format=%cd-%h" "--date=format:%Y.%m.%d")}
HERE=$(pwd)

# If not specified, build the package as `kumomta-dev`.
# When we're building from a tag we'll set 'DEB_NAME=kumomta'
DEB_NAME=${DEB_NAME:-kumomta-dev}
CONFLICTS=kumomta
[[ ${DEB_NAME} == "kumomta" ]] && CONFLICTS=kumomta-dev

KUMO_DEB_VERSION=$(echo ${TAG_NAME} | tr - _)
distroid=$(sh -c "source /etc/os-release && echo \$ID" | tr - _)
distver=$(sh -c "source /etc/os-release && echo \$VERSION_ID" | tr - _)
export SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct)

rm -rf pkg || true
trap "rm -rf pkg" EXIT

mkdir -p pkg/debian/usr/bin pkg/debian/DEBIAN pkg/debian/usr/share/{applications,wezterm}
cat > pkg/debian/control <<EOF
Package: wezterm
Conflicts: ${CONFLICTS}
Version: ${DEB_NAME}
Architecture: $(dpkg-architecture -q DEB_BUILD_ARCH_CPU)
Maintainer: Wez Furlong <wez@wezfurlong.org>
Section: utils
Priority: optional
Homepage: https://github.com/kumomta/kumomta
Description: A high performance, modern MTA
Source: https://github.com/kumomta/kumomta
EOF

install -Dsm755 -t pkg/debian/usr/bin target/release/kumod
install -Dsm755 -t pkg/debian/usr/bin target/release/traffic-gen

deps=$(cd pkg && dpkg-shlibdeps -O -e debian/usr/bin/*)
mv pkg/debian/control pkg/debian/DEBIAN/control
sed -i '/^Source:/d' pkg/debian/DEBIAN/control  # The `Source:` field needs to be valid in a binary package
echo $deps | sed -e 's/shlibs:Depends=/Depends: /' >> pkg/debian/DEBIAN/control
cat pkg/debian/DEBIAN/control

debname=${DEB_NAME}.$distro$distver
fakeroot dpkg-deb --build pkg/debian $debname.deb

