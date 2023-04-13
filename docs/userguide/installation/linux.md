## Installing on Linux

Pre-built releases are available for CentOS 7, Rocky Linux 8/9, and Ubuntu 20.04/22.04.

A repository is provided to ease installation on supported platforms.

The install instructions for supported platforms are shown below. If your platform is not listed, you can [build from source](source.md).

=== "CentOS7"

    !!! note
        Note that Red Hat full support for RHEL 7 [ended in August
        2019](https://access.redhat.com/support/policy/updates/errata#Retired_Life_Cycle_Dates)
        and CentOS 7 full support [ended in August
        2020](https://wiki.centos.org/About/Product).

        We recommend upgrading to a newer OS as soon as possible.

    ```console
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

## Creating the initial config
KumoMTA is now installed, but it requires a configuration policy so it knows how to behave.
The config is written in Lua and should live in /opt/kumomta/etc/policy. It **MUST** be named `init.lua` in order to work with systemctl services, so you should start by editing a file at `/opt/kumomta/etc/policy/init.lua` and populate it with at least the minimal config shown below.  Alternately, there is a more substantial config sample [HERE](https://docs.kumomta.com/userguide/configuration/example/), but you must save it as `init.lua`.

```lua
--[[
########################################################
  KumoMTA minimal Send Policy
  (Rename this to init.lua for systemd automation)
  This config policy defines KumoMTA with a minimal
  set of modifications from default.
  Please read the docs at https://docs.kumomta.com/
  For detailed configuration instructions.
########################################################
]]
--
local kumo = require 'kumo'
--[[ Start of INIT section ]]
--

kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:25',
    -- The following intentionally limits outbound trafic for your protection.
    -- Alter this only after reading the documentation.
    max_messages_per_connection = 100,
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:8000',
  }

  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumomta/data',
  }

  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumomta/meta',
  }

  kumo.configure_local_logs {
    log_dir = '/var/log/kumomta',
  }
end)
--[[ End of INIT Section ]]
--

--[[ Start of Non-INIT level config ]]
--
-- PLEASE read https://docs.kumomta.com/ for extensive documentation on customizing this config.
--[[ End of Non-INIT level config ]]
--
```

## Starting KumoMTA
To start KumoMTA you can use the systemd service or start manually.

With systemd:
```console
$ sudo systemctl start kumomta
```

To ensure it survives a restart:
```console
$ sudo systemctl enable kumomta
```

To start manually in the foreground, (Use the service above it you want it in the background)
```console
$ sudo /opt/kumomta/sbin/kumod \
   --policy /opt/kumomta/etc/policy/init.lua \
   --user kumod
```

