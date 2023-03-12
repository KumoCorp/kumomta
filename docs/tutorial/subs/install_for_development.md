# Installing KumoMTA for Development

## Prepare your environment

Read the [Environmental considerations](https://github.com/kumomta/kumomta/blob/main/docs/guide/subs/environment_consideration.md) before proceeding.  You will need a suitably sized server with all of the prerequisites in order to be successful.

## Step by Step

The commands below will install as a local user.
You can either just execute the installer script (kumoinstall.sh), or follow the steps below manually (same thing).

At a minimum, you will need to install some dev tools and other glue before starting.
And you should make sure you have all the latest patches first too.

### In Rocky, CentOS, Alma, and (likely) any other dnf supporting OS

```bash
sudo dnf clean all
sudo dnf update -y
sudo dnf group install -y "Development Tools"
sudo dnf install -y libxml2 libxml2-devel clang telnet git

```

### Special case for CentOS7

Note that Red Hat full support for RHEL 7 [ended in August 2019](https://access.redhat.com/support/policy/updates/errata#Retired_Life_Cycle_Dates) and CentOS 7 full support [ended in August 2020](https://wiki.centos.org/About/Product)

This is long and complicated and only relevent if you plan to use Cento7 AND need the full build for development.  

If that describes you, then you can follow this to prepage your system, then come back to install Rust and the KumoMTA repo.

[Special Instructions for Centos7](https://github.com/kumomta/kumomta/blob/main/docs/guide/subs/special_for_centos7)

If you just want to run it in CentOS7, we built and RPM for you [on this page](https://github.com/kumomta/kumomta/blob/main/docs/guide/subs/install_for_production_use.md).

### In Ubuntu

```bash
sudo apt-get -y update
sudo apt-get -y upgrade
sudo apt-get install -y build-essential
sudo apt-get install -y cmake make gcc clang llvm telnet git apt-utils
```

### In OpenSuse (Note that SLES does not appear to have an available clang package)

```bash
sudo zypper refresh
sudo zypper update -y
sudo zypper install -y cmake make gcc clang llvm telnet git gcc-c++

```

## Install Rust

If you are using a priviledged user, drop back to your non-priviledged user first.

```bash
cd 
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.profile
source ~/.cargo/env
rustc -V

```

## Get the repo and build it

```git clone https://github.com/kumomta/kumomta.git```

```
cd kumomta
KUMOD_LOG=kumod=trace cargo run -p kumod -- --policy simple_policy.lua

```

In the above you are telling Cargo to run the Rust compiler to build an optimized release version and package it as kumod, then execute kumod using the policy file called simple_policy.lua.

## Using KumoMTA in a Docker container

To build a lightweight alpine-based docker image:
First ensure docker is actually installed in your server instance.

- In Ubuntu, Debian, and other Debian APT package management systems:
  - `sudo apt install -y docker.io apt-utils`

- In Rocky, Alma, and any other DNF package manager system
  - `sudo dnf install -y`

Then build the docker image from the repo root (~/kumomta)

`sudo ./docker/kumod/build-docker-image.sh`

```bash
docker image ls kumomta/kumod
REPOSITORY      TAG       IMAGE ID       CREATED         SIZE
kumomta/kumod   latest    bbced15ff4d1   3 minutes ago   116MB
```

You can then run that image; this invocation mounts the kumo
src dir at `/config` and then the `KUMO_POLICY` environment
variable is used to override the default `/config/policy.lua`
path to use the SMTP sink policy script [sink.lua](https://github.com/kumomta/kumomta/blob/main/sink.lua),
which will accept and discard all mail:

```bash
$ sudo docker run --rm -p 2025:25 \
    -v .:/config \
    --name kumo-sink \
    --env KUMO_POLICY="/config/sink.lua" \
    kumomta/kumod
    
```

If you are planning to just "use" KumoMTA and not develop against it, then you are better off using a prebuilt Docker Image.  See the next section for more on that.

You can add debugging output by adding `KUMOD_LOG=kumod=trace` in the environment when you start kumod.

Then follow the rest above...

## Run as root after the build

Once you have built the package you can run as root separately like this:

```bash
sudo ~/kumomta/target/release/kumod --policy simple_policy.lua
```

## Getting the latest

If you want to always be runniung the latest version, start in the instal directory ( IE: ~/kumomta/ ) then pull and build the latest.

`git pull`

Then `KUMOD_LOG=kumod=trace cargo run -p kumod -- --policy simple_policy.lua`
