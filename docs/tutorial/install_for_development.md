# Installing KumoMTA for Development

## Prepare your environment

Read the [Environmental considerations](https://github.com/kumomta/kumomta/blob/main/docs/tutorial/environment_consideration.md) before proceeding.  You will need a suitably sized server with all of the prerequisites in order to be successful.

## Step by Step

The commands below assume you have already followed the steps in [System Preparation](./system_preparation.md) and will install as a local user.
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


You can add debugging output by adding `KUMOD_LOG=kumod=trace` in the environment when you start kumod.


## Run as root after the build

Once you have built the package you can run as root separately like this:

```bash
sudo ~/kumomta/target/release/kumod --policy simple_policy.lua
```

## Getting the latest

If you want to always be runniung the latest version, start in the instal directory ( IE: ~/kumomta/ ) then pull and build the latest.

`git pull`

Then `KUMOD_LOG=kumod=trace cargo run -p kumod -- --policy simple_policy.lua`
