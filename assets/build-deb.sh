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
distro=$(lsb_release -is 2>/dev/null || sh -c "source /etc/os-release && echo \$NAME")
distver=$(lsb_release -rs 2>/dev/null || sh -c "source /etc/os-release && echo \$VERSION_ID")
export SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct)

rm -rf pkg || true
trap "rm -rf pkg" EXIT

mkdir -p pkg/debian/ pkg/debian/DEBIAN
cat > pkg/debian/control <<EOF
Package: ${DEB_NAME}
Conflicts: ${CONFLICTS}
Version: ${TAG_NAME}
Architecture: $(dpkg-architecture -q DEB_BUILD_ARCH_CPU)
Maintainer: Wez Furlong <wez@wezfurlong.org>
Section: utils
Priority: optional
Homepage: https://github.com/kumomta/kumomta
Description: A high performance, modern MTA
Source: https://github.com/kumomta/kumomta
EOF

./assets/install.sh pkg/debian/opt/kumomta

cat > pkg/debian/preinst <<EOF
#!/bin/sh
getent group kumod >/dev/null || groupadd --system kumod
getent passwd kumod >/dev/null || \
    useradd --system -g kumod -d /var/spool/kumod -s /sbin/nologin \
    -c "Service account for kumomta" kumod

for dir in /opt/kumomta/etc /opt/kumomta/etc/policy ; do
  [ -d "\$dir" ] || install -d --mode 665 --owner kumod --group kumod \$dir
done

for dir in /var/spool/kumomta /var/log/kumomta /opt/kumomta/etc/dkim ; do
  [ -d "\$dir" ] || install -d --mode 2770 --owner kumod --group kumod \$dir
done

exit 0
EOF

deps=$(cd pkg && dpkg-shlibdeps -O -e debian/opt/kumomta/*bin/*)
mv pkg/debian/control pkg/debian/DEBIAN/control
sed -i '/^Source:/d' pkg/debian/DEBIAN/control  # The `Source:` field needs to be valid in a binary package
echo $deps | sed -e 's/shlibs:Depends=/Depends: /' >> pkg/debian/DEBIAN/control
cat pkg/debian/DEBIAN/control

debname=${DEB_NAME}.$distro$distver
fakeroot dpkg-deb --build pkg/debian $debname.deb

