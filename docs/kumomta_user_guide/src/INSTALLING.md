# Installing KumoMTA

## Using a Docker container for commercial mailing
Skip the instructions below and just download a Docker container as described in the docs.


## Installing for active development
Deploy a suitable server (instance).  
So far this is tested on Rocky 8, ...

Note that in order for KumoMTA to bind to port 25 for outbound mail, it must be run as a privileged user.
The commands below will install as a local user.

Install Rust

```curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh```

Install git

```sudo dnf install -y git```

Get the repo

```git clone https://github.com/wez/kumomta.git```

Build it

```
cd kumomta
cargo run --release -p kumod -- --policy simple_policy.lua
```


In the above you are telling Cargo to run the Rust compiler to build an optimized release version and package it as kumod, then execute kumod using the policy file called simple_policy.lua.

If you are planning to just "use" KumoMTA and not develop against it, then you are better off using a Docker Image.  See above section for more on that. 

You can add debugging output by adding KUMOD_LOG=kumod=trace in the environment when you start kumod.

## Run as root after the build

Once you have built the package you can run as root separately like this:
sudo ~/kumomta/target/release/kumod --policy simple_policy.lua


