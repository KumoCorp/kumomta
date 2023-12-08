#!/bin/sh
set -e

NOTFOUND=0

have_command() {
  command -v $1 >/dev/null
}

if test -z "$SUDO"; then
  if have_command 'sudo'; then
    SUDO="sudo"
  elif have_command 'doas'; then
    SUDO="doas"
  fi
fi

alpine_deps() {
  APK="$SUDO apk"
  $APK add \
    'alpine-sdk' \
    'bash' \
    'build-base' \
    'coreutils' \
    'openssl-dev' \
    'pkgconf' \
    'python3' \
    'zlib-dev' \
    'zstd-dev'

  if ! have_command 'cargo'; then
    $APK add 'cargo'
  fi
}

fedora_deps() {
  if have_command 'dnf'; then
    YUM="$SUDO dnf"
  elif have_command 'yum'; then
    YUM="$SUDO yum"
  else
    echo "No idea what package manager to use, sorry! (perhaps 'dnf' or 'yum' is not in \$PATH?)"
    return 1
  fi
  # perl stuff moved around in different versions of the distro.
  # Make a soft attempt under this name.
  $YUM install -y 'perl-FindBin' 'perl-File-Compare' || true
  if ! have_command 'curl' ; then
    # Some systems have curl-minimal which won't tolerate us
    # trying to install curl, so only try to install if we
    # don't have it already
    $YUM install -y 'curl'
  fi
  $YUM install -y \
    'clang-devel' \
    'cmake' \
    'gcc' \
    'gcc-c++' \
    'git' \
    'make' \
    'openssl-devel' \
    'pkg-config' \
    'python3' \
    'python3-pip' \
    'redis' \
    'rpm-build' \
    'rpm-sign' \
    'telnet'
}

amazon_deps() {
  if have_command 'dnf'; then
    YUM="$SUDO dnf"
  elif have_command 'yum'; then
    YUM="$SUDO yum"
  else
    echo "No idea what package manager to use, sorry! (perhaps 'dnf' or 'yum' is not in \$PATH?)"
    return 1
  fi
  if ! have_command 'curl' ; then
    # Some systems have curl-minimal which won't tolerate us
    # trying to install curl, so only try to install if we
    # don't have it already
    $YUM install -y 'curl'
  fi

  # Amazon Linux 2 has some legacy openssl stuff to workaround
  case $VERSION in
    2)
      $YUM remove 'openssl' || true
      $YUM install -y 'openssl11-devel'
      ;;
    *)
      $YUM install -y 'openssl-devel'
      ;;
  esac

  $YUM install -y \
    'binutils' \
    'ca-certificates' \
    'clang-devel' \
    'cmake' \
    'gcc' \
    'gcc-c++' \
    'glibc-devel' \
    'git' \
    'kernel-devel' \
    'kernel-headers' \
    'make' \
    'pkgconfig' \
    'python3' \
    'python3-pip' \
    'rpm-build' \
    'rpm-sign'
}

mariner_deps() {
  if have_command 'dnf'; then
    YUM="$SUDO dnf"
  elif have_command 'yum'; then
    YUM="$SUDO yum"
  else
    echo "No idea what package manager to use, sorry! (perhaps 'dnf' or 'yum' is not in \$PATH?)"
    return 1
  fi
  # perl stuff moved around in different versions of the distro.
  # Make a soft attempt under this name.
  $YUM install -y 'perl-FindBin' 'perl-File-Compare' || true
  if ! have_command 'curl' ; then
    # Some systems have curl-minimal which won't tolerate us
    # trying to install curl, so only try to install if we
    # don't have it already
    $YUM install -y 'curl'
  fi
  $YUM install -y \
    'binutils' \
    'ca-certificates' \
    'clang-devel' \
    'cmake' \
    'gcc' \
    'gcc-c++' \
    'glibc-devel' \
    'git' \
    'kernel-devel' \
    'kernel-headers' \
    'make' \
    'openssl-devel' \
    'pkg-config' \
    'python3' \
    'python3-pip' \
    'redis' \
    'rpm-build' \
    'rpm-sign'
}

suse_deps() {
  ZYPPER="$SUDO zypper"
  $ZYPPER install -yl \
    'clang' \
    'cmake' \
    'gcc' \
    'gcc-c++' \
    'git' \
    'libopenssl-devel' \
    'llvm' \
    'make' \
    'pkg-config' \
    'python3' \
    'redis' \
    'rpm-build' \
    'telnet'
}

debian_deps() {
  APT="$SUDO apt-get"
  $APT install -y --no-install-recommends \
    'bsdutils' \
    'cmake' \
    'dpkg-dev' \
    'fakeroot' \
    'gcc' \
    'g++' \
    'libssl-dev' \
    'lsb-release' \
    'pkg-config' \
    'python3' \
    'redis' \
    'llvm-dev' \
    'libclang-dev' \
    'clang'
}

arch_deps() {
  PACMAN="$SUDO pacman"
  $PACMAN -S --noconfirm --needed \
    'base-devel' \
    'cargo' \
    'clang' \
    'cmake' \
    'git' \
    'pkgconf' \
    'python3' \
    'rust'
}

bsd_deps() {
  PKG="$SUDO pkg"
  $PKG install -y \
    'cmake' \
    'curl' \
    'gcc' \
    'gettext' \
    'git' \
    'gmake' \
    'openssl' \
    'pkgconf' \
    'python3' \
    'rust' \
    'z' \
    'zip'
}

gentoo_deps() {
  portageq envvar USE | xargs -n 1 | grep '^X$' \
  || (echo 'X is not found in USE flags' && exit 1)
  EMERGE="$SUDO emerge"
  for pkg in \
    'cmake' \
    'openssl' \
    'dev-vcs/git' \
    'pkgconf' \
    'python'
  do
	  equery l "$pkg" > /dev/null || $EMERGE --select $pkg
  done
}

void_deps() {
  XBPS="$SUDO xbps-install"
  $XBPS -S \
    'gcc' \
    'pkgconf' \
    'fontconfig-devel' \
    'openssl-devel'

  if ! have_command 'cargo'; then
    $XBPS -S 'cargo'
  fi
}

solus_deps() {
  EOPKG="$SUDO eopkg"
  $EOPKG install -y -c system.devel
}

fallback_method() {
  if test -e /etc/alpine-release; then
    alpine_deps
  elif test -e /etc/centos-release || test -e /etc/fedora-release || test -e /etc/redhat-release; then
    fedora_deps
  elif test -e /etc/debian_version; then
    debian_deps
  elif test -e /etc/arch-release; then
    arch_deps
  elif test -e /etc/gentoo-release; then
    gentoo_deps
  elif test -e /etc/solus-release; then
    solus_deps
  elif have_command 'lsb_release' && test "$(lsb_release -si)" = "openSUSE"; then
    suse_deps
  fi

  # OSTYPE is set by bash
  case $OSTYPE in
    darwin*|msys)
      echo "skipping darwin*/msys"
    ;;
    freebsd*)
      bsd_deps
    ;;
    ''|linux-gnu)
      # catch and known OSTYPE
      echo "\$OSTYPE is '$OSTYPE'"
    ;;
    *)
      NOTFOUND=1
      return 1
    ;;
  esac
  return 0
}

if test -e /etc/os-release; then
  . /etc/os-release
fi

case $ID in
  centos|fedora|rhel)
    fedora_deps
  ;;
  alpine)
    alpine_deps
  ;;
  *suse*)
    suse_deps
  ;;
  debian|ubuntu)
    debian_deps
  ;;
  freebsd) # available since 13.0
    bsd_deps
  ;;
  arch|artix)
    arch_deps
  ;;
  gentoo)
    gentoo_deps
  ;;
  void)
    void_deps
  ;;
  solus)
    solus_deps
  ;;
  mariner)
    mariner_deps
  ;;
  amzn)
    amazon_deps
  ;;
  *)
    echo "Couldn't find OS by ID, found ID: $ID"
    echo "Fallback to detecting '/etc/<name>-release'"
    fallback_method
    if ! test $? -eq 0; then
      if ! test $NOTFOUND -eq 0; then
        echo "Couldn't identify OS through '/etc/<name>-release'"
      fi
      exit 1
    fi
  ;;
esac

if ! test $NOTFOUND -eq 0; then
  echo "Please contribute the commands to install the deps for:"
  if have_command 'lsb_release'; then
    lsb_release -ds
  elif test -e /etc/os-release; then
    cat /etc/os-release
  else
    echo "Couldn't recognise system"
  fi
  exit 1
fi

if ! have_command 'rustc'; then
  echo "Rust is not installed!"
  echo "Please see https://docs.kumomta.com/userguide/installation/source/ for installation instructions"
  exit 1
fi
