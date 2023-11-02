# Building From Source

If pre-built binaries are not provided for your system of choice, of if you'd
like try your hand at extending KumoMTA, you'll need to build it from source.

If you are on Ubuntu or Rocky Linux and just want to try KumoMTA, rather than
build from source we recommend that you follow the instructions in the [Installing on Linux](linux.md) section.

## Prepare your environment

Read the [Environmental considerations](environment.md) before proceeding.  You
will need a suitably sized server with all of the prerequisites in order to be
successful.

In addition, you will need to install some development packages.

## Obtain The Code

You will need `git`:

=== "RPM based systems"
    ```console
    $ sudo dnf install -y git
    ```

=== "APT based systems"
    ```console
    $ sudo apt install -y git curl
    ```

Then clone the repo:

```console
$ git clone https://github.com/KumoCorp/kumomta.git
$ cd kumomta
```

## Install Dependencies

The `get-deps.sh` script in the repo knows how to install dependencies for
various systems; you should run it the first time you clone the repo,
and may need to run it after running a pull to update the repo in the future:

```console
$ ./get-deps.sh
```

!!! note
    `get-deps.sh` will install the various deps, but will complain
    that rust is not installed the first time that you run it.
    You can ignore that error as the very next step is to install
    rust.

## Install Rust

You will need the Rust compiler to build KumoMTA.

We strongly recommend using [rustup](https://rustup.rs/) to install and manage
your Rust compiler. While some distributions offer a version of the Rust compiler,
it is often outdated.

If you are using a priviledged user, drop back to your non-priviledged user first:

```console
$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
$ source ~/.cargo/env
```

## Building KumoMTA

With all the dependencies available, the actual build process is very simple:

```console
$ cargo build --release
```

This will build everything, leaving the binaries in the `target/release`
directory in the repo.

## Building your own package

There are scripts to build out packages in either RPM or DEB format.
If you're running on such a system, we recommend building and installing from
a package built by our scripts, as those packages will best encapsulate how
we intend for KumoMTA to be installed and operated.

These scripts will produce a package file that you are then free to install
either locally or on a target system elsewhere.

=== "RPM based systems"

    ```console
    $ assets/build-rpm.sh
    ```

    You can find the generated rpm in a directory maintained by rpmbuild; that is
    usually `~/rpmbuild/RPMS/x86_64`, but some environments use a different
    location, so the example below uses `rpm --eval` to obtain the correct location:

    ```console
    $ rpm --eval '%{_rpmdir}/%{_arch}'
    /home/USER/rpmbuild/RPMS/x86_64
    $ ls $(rpm --eval '%{_rpmdir}/%{_arch}')/kumo*.rpm
    /home/USER/rpmbuild/RPMS/x86_64/kumomta-dev-2023.10.24.112314_f8aaa6f1-1.fedora38.x86_64.rpm
    ```

    You can install it directly if you wish:

    ```console
    $ sudo rpm -Uvh $(ls $(rpm --eval '%{_rpmdir}/%{_arch}')/kumo*.rpm | tail -1)
    ```

=== "APT based systems"

    ```console
    $ assets/build-deb.sh
    $ ls *.deb
    kumomta-dev.2023.10.24.112314.f8aaa6f1.Ubuntu22.04.deb
    ```

    You can install it directly if you wish:

    ```console
    $ sudo apt-get install -y ./kumomta*.deb
    ```

## Installing from source

If RPM or DEB is not suitable for your environment for some reason, you can install
the various components "by hand".  We recommend installing to `/opt/kumomta` so that
various product defaults continue to operate as intended.

### Pre-req: service account

The default service account assumed by the `kumod` process is the `kumod` user.

You can create the account manually like this:

```console
$ sudo useradd --system -g kumod -d /var/spool/kumod -s /sbin/nologin \
    -c "Service account for kumomta" kumod
```

### Directory Structure

Take care with the ownership and permissions on the various directories,
in order to avoid deploying with an insecure configuration:

```console
$ sudo install -d --mode 755 --owner kumod --group kumod /opt/kumomta/sbin
$ sudo install -d --mode 755 --owner kumod --group kumod /opt/kumomta/etc
$ sudo install -d --mode 755 --owner kumod --group kumod /opt/kumomta/etc/policy
$ sudo install -d --mode 2770 --owner kumod --group kumod /opt/kumomta/etc/dkim
$ sudo install -d --mode 2770 --owner kumod --group kumod /var/spool/kumomta
$ sudo install -d --mode 2770 --owner kumod --group kumod /var/log/kumomta
```

The executables:

```console
$ for bin in validate-shaping tsa-daemon \
    proxy-server kumod kcli traffic-gen tailer ; do
  install -Dsm755 target/release/$bin -t /opt/kumomta/sbin
done
```

The helpers and other assets:

```console
$ sudo mkdir -p /opt/kumomta/share/bounce_classifier /opt/kumomta/share/policy-extras
$ sudo install -Dm644 assets/bounce_classifier/* -t /opt/kumomta/share/bounce_classifier
$ sudo install -Dm644 assets/policy-extras/*.lua -t /opt/kumomta/share/policy-extras
$ sudo install -Dm644 assets/policy-extras/*.toml -t /opt/kumomta/share/policy-extras
```

The example/starter configuration files can be installed like this; you may wish
to skip this set and just deploy your own configuration, as discussed in the
section below:

```console
$ sudo install -Dm644 assets/init.lua -T /opt/kumomta/etc/policy/init.lua
$ sudo install -Dm644 assets/tsa_init.lua -T /opt/kumomta/etc/policy/tsa_init.lua
```

### Systemd Service

If you wish to use systemd to manage `kumod` and/or `tsa-daemon`, you can find
the `.service` files in the `assets` directory.  Precisely where these files
are deployed varies a little depending on your distribution, so copyable
instructions for that are not currently provided here.

## Creating the initial config

KumoMTA is now installed, but it requires a policy configuration script so it
knows how to behave.  The policy config is written in Lua and should live in
`/opt/kumomta/etc/policy/init.lua` in order to work with the systemd service
definition.

Both the from-package and from-source instructions above will pre-populate
that file with the basic configuration that is reproduced below.
Alternately, there is a more substantial config sample
[HERE](https://docs.kumomta.com/userguide/configuration/example/), but you must
save it as `/opt/kumomta/etc/policy/init.lua`.

```lua title="/opt/kumomta/etc/policy/init.lua"
--8<-- "init.lua"
```

## Running kumod

If you are not using systemd to manage the service, then you will need to
use some other way to launch kumod.  This section shows how you might launch
it manually, so that you understand how to automatate/manage this for yourself
in your chosen environment:

```console
$ sudo /opt/kumomta/sbin/kumod \
   --policy /opt/kumomta/etc/policy/init.lua \
   --user kumod
```

Using `sudo` (or otherwise spawning as root) to launch the process allows
binding to privileged ports, such as port 25, so that you can accept incoming
mail on the standard port.

When launched with root privileges, `kumod` requires a service account to
switch to after it has bound privileged ports, in order to avoid running in a
dangerously insecure mode: you do not want public internet traffic
connecting directly to a privileged process!

The `--policy` argument specifies the path to your `init.lua` configuration.

The `--user` argument specifies the name of the service account to use
instead of running as root.

## Running from your source directory

!!! note
    This section is intended for people that are developing kumomta
    itself, rather than people that just want to install and use kumomta

This command will bring `kumod` up to date (in case you made changes), and then try to launch it:

```console
$ cargo run --release -p kumod -- --policy /opt/kumomta/etc/policy/init.lua
```

You can run as root using port 25, in the foreground, with this:

```console
$ cargo build --release -p kumod && \
  sudo target/release/kumod \
     --policy /opt/kumomta/etc/policy/init.lua \
     --user kumod
```

## Keeping the source up to date

To synchronize your repo with the latest changes in the `main` branch:

```console
$ cd kumomta
$ git pull --rebase
$ ./get-deps.sh
$ cargo build --release
```
Note that this builds the new files in `target/release/`.  If you installed binaries to /opt/kumomta/sbin then you will want to follow the instructions above to [build your own package](source.md/#building-your-own-package) and update the files in `/opt/kumomta/`.
