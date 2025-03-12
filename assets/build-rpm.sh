#!/bin/bash
set -x
set -e
TAG_NAME=${TAG_NAME:-$(git -c "core.abbrev=8" show -s "--format=%cd-%h" "--date=format:%Y.%m.%d")}
HERE=$(pwd)

# If not specified, build the rpm as `kumomta-dev`.
# When we're building from a tag (REF_TYPE is set to tag) we set 'RPM_NAME=kumomta'
[[ "${REF_TYPE}" == "tag" || "${CI_PIPELINE_EVENT}" == "tag" ]] && RPM_NAME=kumomta
RPM_NAME=${RPM_NAME:-kumomta-dev}

[[ ${RPM_NAME} == "kumomta-dev" ]] && export KEEP_DEBUG=yes
if [ ${KEEP_DEBUG} == "yes" ] ; then
  KEEP_DEBUG_SPEC=$(cat <<'EOT'
%global debug_package %{nil}
# Disable stripping
%global __strip /bin/true
%global __objdump /bin/true

# Tone down the compression level.
# It takes too long with the defaults on rocky9
# https://stackoverflow.com/a/10255406/149111
%define _source_payload w6.gzdio
%define _binary_payload w6.gzdio
# uncompressed is 1.3GB
# gzip 9 gives 370M in 2mins
# gzip 6       372M in 40s
# gzip 4       385M in 25s
# default      267M in 4mins
EOT
)
fi

CONFLICTS=kumomta
[[ ${RPM_NAME} == "kumomta" ]] && CONFLICTS=kumomta-dev

KUMO_RPM_VERSION=$(git -c "core.abbrev=8" show -s "--format=%cd_%h" "--date=format:%Y.%m.%d.%H%M%S")
distroid=$(sh -c "source /etc/os-release && echo \$ID" | tr - _)
distver=$(sh -c "source /etc/os-release && echo \$VERSION_ID" | tr - _)
export SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct)

spec=$(mktemp)
trap "rm ${spec}" "EXIT"

cat > $spec <<EOF
Name: ${RPM_NAME}
Conflicts: ${CONFLICTS}
Version: ${KUMO_RPM_VERSION}
Release: 1.${distroid}${distver}
Packager: Wez Furlong <wez@wezfurlong.org>
License: MIT
URL: https://kumomta.com
Summary: A high performance, modern MTA.
Requires(pre): shadow-utils

${KEEP_DEBUG_SPEC}

%description
A high performance, modern MTA.

%build
echo "Doing the build bit here"

%post

if [ ! -f "/opt/kumomta/etc/policy/init.lua" ] ; then
  # Create initial policy script
  cp /opt/kumomta/share/minimal-init.lua /opt/kumomta/etc/policy/init.lua
fi
if [ ! -f "/opt/kumomta/etc/policy/tsa_init.lua" ] ; then
  # Create initial policy script
  cp /opt/kumomta/share/minimal-tsa_init.lua /opt/kumomta/etc/policy/tsa_init.lua
fi

if [ \$1 -eq 1 ] && [ -x "/usr/lib/systemd/systemd-update-helper" ]; then
    # Initial installation
    /usr/lib/systemd/systemd-update-helper install-system-units kumomta.service kumo-tsa-daemon.service || :
fi

%preun

if [ \$1 -eq 0 ] && [ -x "/usr/lib/systemd/systemd-update-helper" ]; then
    # Package removal, not upgrade
    /usr/lib/systemd/systemd-update-helper remove-system-units kumomta.service kumo-tsa-daemon.service || :
fi

%postun

if [ \$1 -ge 1 ] && [ -x "/usr/lib/systemd/systemd-update-helper" ]; then
    # Package upgrade, not uninstall
    /usr/lib/systemd/systemd-update-helper mark-restart-system-units kumomta.service kumo-tsa-daemon.service || :
fi

%pre
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

%install
set -x
cd ${HERE}
./assets/install.sh %{buildroot}/opt/kumomta
mkdir -p %{buildroot}/usr/lib/systemd/system
install -Dm644 ./assets/kumomta.service -t %{buildroot}/usr/lib/systemd/system
install -Dm644 ./assets/kumo-tsa-daemon.service -t %{buildroot}/usr/lib/systemd/system

%files
/opt/kumomta/sbin/kcli
/opt/kumomta/sbin/kumod
/opt/kumomta/sbin/proxy-server
/opt/kumomta/sbin/resolve-site-name
/opt/kumomta/sbin/resolve-queue-config
/opt/kumomta/sbin/resolve-shaping-domain
/opt/kumomta/sbin/tailer
/opt/kumomta/sbin/tls-probe
/opt/kumomta/sbin/toml2jsonc
/opt/kumomta/sbin/traffic-gen
/opt/kumomta/sbin/tsa-daemon
/opt/kumomta/sbin/validate-shaping
/opt/kumomta/sbin/accounting.sh
/opt/kumomta/sbin/explain-throttle
/opt/kumomta/share/bounce_classifier/*.toml
/opt/kumomta/share/minimal-init.lua
/opt/kumomta/share/minimal-tsa_init.lua
/opt/kumomta/share/policy-extras/*.lua
/opt/kumomta/share/policy-extras/*.toml
/opt/kumomta/share/community/*.toml
/usr/lib/systemd/system/kumomta.service
/usr/lib/systemd/system/kumo-tsa-daemon.service
EOF

/usr/bin/rpmbuild -bb $spec --verbose
