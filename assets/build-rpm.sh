#!/bin/bash
set -x
set -e
TAG_NAME=${TAG_NAME:-$(git -c "core.abbrev=8" show -s "--format=%cd-%h" "--date=format:%Y.%m.%d")}
HERE=$(pwd)

# If not specified, build the rpm as `kumomta-dev`.
# When we're building from a tag we'll set 'RPM_NAME=kumomta'
RPM_NAME=${RPM_NAME:-kumomta-dev}
CONFLICTS=kumomta
[[ ${RPM_NAME} == "kumomta" ]] && CONFLICTS=kumomta-dev

KUMO_RPM_VERSION=$(echo ${TAG_NAME} | tr - _)
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

%description
A high performance, modern MTA.

%build
echo "Doing the build bit here"

%pre
getent group kumod >/dev/null || groupadd --system kumod
getent passwd kumod >/dev/null || \
    useradd --system -g kumod -d /var/spool/kumod -s /sbin/nologin \
    -c "Service account for kumomta" kumod

for dir in /var/spool/kumomta /var/log/kumomta ; do
  [ -d "\$dir" ] || install -d --mode 2770 --owner kumod --group kumod \$dir
done

exit 0

%install
set -x
cd ${HERE}
./assets/install.sh %{buildroot}/opt/kumomta

%files
/opt/kumomta/sbin/kumod
/opt/kumomta/sbin/traffic-gen
/opt/kumomta/share/bounce_classifier/*.toml
EOF

/usr/bin/rpmbuild -bb $spec --verbose
