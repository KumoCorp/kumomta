#!/bin/bash
set -x
set -e
TAG_NAME=${TAG_NAME:-$(git -c "core.abbrev=8" show -s "--format=%cd-%h" "--date=format:%Y.%m.%d")}
HERE=$(pwd)

# If not specified, build the package as `kumomta-dev`.
# When we're building from a tag (REF_TYPE is set to tag) we set 'DEB_NAME=kumomta'
[[ "${REF_TYPE}" == "tag" || "${CI_PIPELINE_EVENT}" == "tag" ]] && DEB_NAME=kumomta
DEB_NAME=${DEB_NAME:-kumomta-dev}

CONFLICTS=kumomta
[[ ${DEB_NAME} == "kumomta" ]] && CONFLICTS=kumomta-dev

KUMO_DEB_VERSION=$(git -c "core.abbrev=8" show -s "--format=%cd.%h" "--date=format:%Y.%m.%d.%H%M%S")
distro=$(lsb_release -is 2>/dev/null || sh -c "source /etc/os-release && echo \$NAME")
distver=$(lsb_release -rs 2>/dev/null || sh -c "source /etc/os-release && echo \$VERSION_ID")
export SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct)

rm -rf pkg || true
trap "rm -rf pkg" EXIT

mkdir -p pkg/debian/ pkg/debian/DEBIAN
cat > pkg/debian/control <<EOF
Package: ${DEB_NAME}
Conflicts: ${CONFLICTS}
Version: ${KUMO_DEB_VERSION}
Architecture: $(dpkg-architecture -q DEB_BUILD_ARCH_CPU)
Maintainer: Wez Furlong <wez@wezfurlong.org>
Section: utils
Priority: optional
Homepage: https://github.com/KumoCorp/kumomta
Description: A high performance, modern MTA
Source: https://github.com/KumoCorp/kumomta
EOF

./assets/install.sh pkg/debian/opt/kumomta

install -Dm644 ./assets/kumomta.service -t pkg/debian/usr/lib/systemd/system
install -Dm644 ./assets/kumo-tsa-daemon.service -t pkg/debian/usr/lib/systemd/system

cat > pkg/debian/DEBIAN/preinst <<EOF
#!/bin/sh
getent group kumod >/dev/null || groupadd --system kumod
getent passwd kumod >/dev/null || \
    useradd --system -g kumod -d /var/spool/kumod -s /sbin/nologin \
    -c "Service account for kumomta" kumod

for dir in /opt/kumomta/etc /opt/kumomta/etc/policy ; do
  [ -d "\$dir" ] || install -d --mode 755 --owner kumod --group kumod \$dir
done

for dir in /var/spool/kumomta /var/log/kumomta /opt/kumomta/etc/dkim ; do
  [ -d "\$dir" ] || install -d --mode 2770 --owner kumod --group kumod \$dir
done

exit 0
EOF
chmod 0775 pkg/debian/DEBIAN/preinst

cat > pkg/debian/DEBIAN/postinst <<EOF
#!/bin/sh
set -e
if [ "\$1" = "configure" ]; then

    if [ ! -f "/opt/kumomta/etc/policy/init.lua" ] ; then
      # Create initial policy script
      cp /opt/kumomta/share/minimal-init.lua /opt/kumomta/etc/policy/init.lua
    fi
    if [ ! -f "/opt/kumomta/etc/policy/tsa_init.lua" ] ; then
      # Create initial policy script
      cp /opt/kumomta/share/minimal-tsa_init.lua /opt/kumomta/etc/policy/tsa_init.lua
    fi

    if [ -x "/usr/bin/deb-systemd-helper" ]; then
      deb-systemd-helper enable kumomta.service >/dev/null
      deb-systemd-helper enable kumo-tsa-daemon.service >/dev/null
    fi
fi
exit 0
EOF
chmod 0775 pkg/debian/DEBIAN/postinst

cat > pkg/debian/DEBIAN/postrm <<EOF
#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then
    systemctl --system daemon-reload >/dev/null || true
fi
if [ "\$1" = "remove" ]; then
    if [ -x "/usr/bin/deb-systemd-helper" ]; then
        deb-systemd-helper mask kumomta.service >/dev/null
        deb-systemd-helper mask kumo-tsa-daemon.service >/dev/null
    fi
fi

if [ "\$1" = "purge" ]; then
     if [ -x "/usr/bin/deb-systemd-helper" ]; then
        deb-systemd-helper purge kumomta.service >/dev/null
        deb-systemd-helper unmask kumomta.service >/dev/null

        deb-systemd-helper purge kumo-tsa-daemon.service >/dev/null
        deb-systemd-helper unmask kumo-tsa-daemon.service >/dev/null
    fi
fi
exit 0
EOF
chmod 0775 pkg/debian/DEBIAN/postrm

cat > pkg/debian/DEBIAN/prerm <<EOF
#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then
    deb-systemd-helper stop kumomta.service >/dev/null
    deb-systemd-helper stop kumo-tsa-daemon.service >/dev/null
fi
exit 0
EOF
chmod 0775 pkg/debian/DEBIAN/prerm


deps=$(cd pkg && dpkg-shlibdeps -O -e debian/opt/kumomta/*bin/*)
mv pkg/debian/control pkg/debian/DEBIAN/control
sed -i '/^Source:/d' pkg/debian/DEBIAN/control  # The `Source:` field needs to be valid in a binary package
echo $deps | sed -e 's/shlibs:Depends=/Depends: /' >> pkg/debian/DEBIAN/control
cat pkg/debian/DEBIAN/control

debname=${DEB_NAME}.${KUMO_DEB_VERSION}.$distro$distver
find pkg -ls
FAKEROOT=fakeroot
if test "$EUID" -eq 0 ; then
  FAKEROOT=""
fi
$FAKEROOT dpkg-deb --verbose --build pkg/debian $debname.deb

