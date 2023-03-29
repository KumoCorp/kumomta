## Installing on Linux

Pre-built releases are available for CentOS 7, Rocky Linux 8/9, and Ubuntu 20.04/22.04.

A repository is provided to ease installation on supported platforms.

The install instructions for supported platforms are shown below. If your platform is not listed, you can [build from source](source.md).

=== "CentOS7"

!!! note
    Note that Red Hat full support for RHEL 7 [ended in August 2019](https://access.redhat.com/support/policy/updates/errata#Retired_Life_Cycle_Dates) and CentOS 7 full support [ended in August 2020](https://wiki.centos.org/About/Product).

    We recommend upgrading to a newer OS as soon as possible.

```console
$ sudo yum-config-manager --add-repo=\
    https://openrepo.kumomta.com/files/kumomta-centos.repo
$ sudo yum install kumomta-dev
```

=== "Rocky"

```console
$ sudo dnf -y install dnf-plugins-core
$ sudo dnf config-manager --add-repo \
    https://openrepo.kumomta.com/files/kumomta-rocky.repo
$ sudo yum install kumomta-dev
```

=== "Ubuntu 22.04 LTS"

```console
$ sudo apt install -y curl gnupg ca-certificates
$ curl -fsSL https://openrepo.kumomta.com/kumomta-ubuntu-22/public.gpg | sudo gpg --yes --dearmor -o /usr/share/keyrings/kumomta.gpg
$ curl -fsSL https://openrepo.kumomta.com/files/kumomta-ubuntu22.list | sudo tee /etc/apt/sources.list.d/kumomta.list > /dev/null
$ sudo apt update
$ sudo apt install -y kumomta-dev
```

=== "Ubuntu 20.04 LTS"

```console
$ sudo apt install -y curl gnupg ca-certificates
$ curl -fsSL https://openrepo.kumomta.com/kumomta-ubuntu-20/public.gpg | sudo gpg --yes --dearmor -o /usr/share/keyrings/kumomta.gpg
$ curl -fsSL https://openrepo.kumomta.com/files/kumomta-ubuntu20.list | sudo tee /etc/apt/sources.list.d/kumomta.list > /dev/null
$ sudo apt update
$ sudo apt install -y kumomta-dev
```
