# Installing KumoMTA for Development

## Prepare your environment
Deploy a suitable server (instance).  This should have at least 4Gb RAM and 2 cores and 20Gb Storage. In AWS, a t2.medium is adequate for a minimal install.
For performance install, more is better.  

So far this is tested on Rocky 8, ...

Note that in order for KumoMTA to bind to port 25 for outbound mail, it must be run as a privileged user.
Note also that if you are deploying to any public cloud, outbound port 25 is probably blocked by default. If this node specificially needs to send mail directly on port 25 to the public internet, you should request access to the port from the cloud provider.  Some hints are below.

AWS: https://aws.amazon.com/premiumsupport/knowledge-center/ec2-port-25-throttle/

Azure: https://learn.microsoft.com/en-us/azure/virtual-network/troubleshoot-outbound-smtp-connectivity

GCP: https://cloud.google.com/compute/docs/tutorials/sending-mail


## Step by Step

The commands below will install as a local user.
You can either just execute the [installer script](/kumoinstall.sh), or follow the steps below manually (same thing).

At a minimum, you will need to install some dev tools and other glue before starting.

```
sudo dnf group install -y "Development Tools"
sudo dnf install -y libxml2 libxml2-devel clang telnet
```

And you should make sure you have all the latest patches first too.

```
sudo dnf clean all
sudo dnf update -y
```

Install Rust

```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.profile
source ~/.cargo/env
rustc -V
```

Install git

```sudo dnf install -y git```

Get the repo

```git clone https://github.com/kumomta/kumomta.git```

Build it

```
cd kumomta
KUMOD_LOG=kumod=trace cargo run -p kumod -- --policy simple_policy.lua
```


In the above you are telling Cargo to run the Rust compiler to build an optimized release version and package it as kumod, then execute kumod using the policy file called simple_policy.lua.

If you are planning to just "use" KumoMTA and not develop against it, then you are better off using a Docker Image.  See the next section for more on that. 

You can add debugging output by adding KUMOD_LOG=kumod=trace in the environment when you start kumod.

## Run as root after the build

Once you have built the package you can run as root separately like this:
sudo ~/kumomta/target/release/kumod --policy simple_policy.lua


