# Installing on Linux

Pre-built releases are available for CentOS 7, Rocky Linux 8/9, and Ubuntu 20.04/22.04.

A repository is provided to ease installation on supported platforms.

The install instructions for supported platforms are shown below. If your platform is not listed, you can [build from source](source.md).

=== "CentOS7"

    !!! note
        Note that Red Hat full support for RHEL 7 [ended in August
        2019](https://access.redhat.com/support/policy/updates/errata#Retired_Life_Cycle_Dates)
        and CentOS 7 full support [ended in August
        2020](https://wiki.centos.org/About/Product). We ended automated builds for RHEL 7 with the [2023.08.22-4d895015](../../changelog/2023.08.22-4d895015.md) release.

        We recommend upgrading to a newer OS as soon as possible.

    ```console
    $ sudo yum install -y yum-utils
    $ sudo yum-config-manager --add-repo=\
        https://openrepo.kumomta.com/files/kumomta-centos.repo
    $ sudo yum install kumomta
    ```

=== "Rocky"

    ```console
    $ sudo dnf -y install dnf-plugins-core
    $ sudo dnf config-manager --add-repo \
        https://openrepo.kumomta.com/files/kumomta-rocky.repo
    $ sudo yum install kumomta
    ```

=== "Ubuntu 22.04 LTS"

    ```console
    $ sudo apt install -y curl gnupg ca-certificates
    $ curl -fsSL https://openrepo.kumomta.com/kumomta-ubuntu-22/public.gpg | sudo gpg --yes --dearmor -o /usr/share/keyrings/kumomta.gpg
    $ curl -fsSL https://openrepo.kumomta.com/files/kumomta-ubuntu22.list | sudo tee /etc/apt/sources.list.d/kumomta.list > /dev/null
    $ sudo apt update
    $ sudo apt install -y kumomta
    ```

=== "Ubuntu 20.04 LTS"

    ```console
    $ sudo apt install -y curl gnupg ca-certificates
    $ curl -fsSL https://openrepo.kumomta.com/kumomta-ubuntu-20/public.gpg | sudo gpg --yes --dearmor -o /usr/share/keyrings/kumomta.gpg
    $ curl -fsSL https://openrepo.kumomta.com/files/kumomta-ubuntu20.list | sudo tee /etc/apt/sources.list.d/kumomta.list > /dev/null
    $ sudo apt update
    $ sudo apt install -y kumomta
    ```

=== "Amazon Linux 2"

    !!! note
        There are no stable versions available for this platform,
        so if you want a pre-built binary you could consider
        installing a `-dev` build, see instructions below

=== "Amazon Linux 2023"

    !!! note
        There are no stable versions available for this platform,
        so if you want a pre-built binary you could consider
        installing a `-dev` build, see instructions below

## Installing from a Dev Repository

If you want to test the latest additions and improvements to KumoMTA, you can instead install from the dev repository on your platform of choice. The dev repository is rebuilt after each commit to the KumoMTA repository, which means the dev repository will always include the latest changes.

!!! warning
    While we do our best to test all commits, dev repositories should **never** be installed in production environments.

=== "CentOS7"

    !!! note
        Note that Red Hat full support for RHEL 7 [ended in August
        2019](https://access.redhat.com/support/policy/updates/errata#Retired_Life_Cycle_Dates)
        and CentOS 7 full support [ended in August
        2020](https://wiki.centos.org/About/Product).

        We recommend upgrading to a newer OS as soon as possible.

    ```console
    $ sudo yum install -y yum-utils
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

=== "Amazon Linux 2"

    ```console
    $ sudo yum install -y yum-utils
    $ sudo yum-config-manager --add-repo=\
        https://openrepo.kumomta.com/files/kumomta-amazon.repo
    $ sudo yum install kumomta-dev
    ```

=== "Amazon Linux 2023"

    ```console
    $ sudo dnf -y install dnf-plugins-core
    $ sudo dnf config-manager --add-repo \
        https://openrepo.kumomta.com/files/kumomta-amazon2023.repo
    $ sudo yum install kumomta-dev
    ```


## The Initial Config File

KumoMTA is now installed, but it requires a configuration policy so it knows how to behave. The installer creates a minimal configuration policy file at `/opt/kumomta/etc/policy/init.lua` that enables basic localhost relaying and logging.

See the [configuration](../configuration/concepts.md) chapter for more information on creating your own configuration policy.

## Starting KumoMTA

To start KumoMTA using systemd, execute the following command:

```console
$ sudo systemctl start kumomta
```

For additional details on starting KumoMTA, including as a persistent service, see the [Starting KumoMTA](../operation/starting.md) chapter.
