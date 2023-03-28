# KumoMTA - Day 1

This document will outline a typical deployment of KumoMTA starting from scratch. This walkthrough is not a replacement for reading the full documentation, but rather will show how to install and configure in a specific environment as a sample that you can bend to your own needs.

We assume that you iknow Linux, have an AWS account, Github account, and have a working knowledge of how those tools operate. We also assume that you know what an MTA is and why you need one.  If not, you may want to [read this first](https://en.wikipedia.org/wiki/Message_transfer_agent).

## Getting Started
The scenario we are going to emulate is a deployment using Rocky Linux V9 in AWS Public cloud. This will be a single node server having to send about eight million messages a day to the public Internet. The average size of these messages will be 50KB.

## The TL;DR version
If you just want to get this installed and running without exhaustive explanation, follow these steps. This assumes you know what you are doing and just want the high-level info.  The longer version with deeper explanation follows in the next section.

1) Spin up an AWS t2.xlarge (or larger)instance (or any server with at least 4vCPUs, 16Gb RAM, 300Gb Hard Drive)

2) Install Rocky linux 9

3) Update the OS and disable Postfix if needed

```console
sudo dnf clean all
sudo dnf update -y
sudo systemctl stop  postfix.service
sudo systemctl disable postfix.service
```

4) Add the KumoMTA repo to your config manager and yum install it like this:

```console
sudo dnf -y install dnf-plugins-core
sudo dnf config-manager \
    --add-repo \
    https://openrepo.kumomta.com/files/kumomta-rocky.repo
sudo yum install kumomta-dev
```

5) Create a configuration policy in ```/opt/kumomta/etc/policy/example.lua``` based on the example at [ https://docs.kumomta.com/userguide/configuration/example/](https://docs.kumomta.com/userguide/configuration/example/)
Hint, you can copy and paste that into a new file and edit the necessary parts.
You should either create dkim keys or comment out the dkim signing portion for now.

6) Run it with : 
```console
sudo /opt/kumomta/sbin/kumod --policy \
  /opt/kumomta/etc/policy/example.lua --user kumod&
```

And you are done.  KumoMTA will now be installed and running the example configuration from ```/opt/kumomta/sbin/kumod```.  The & pushes the running process to the background, type 'fg' to bring it forward again.

## The Longer Version
This page described a situation where you already have a fully prepared server/instance and just needed basic install instructions.  Read on to the next section to look at server selection and sizing, OS preparation, installing and testing it with more detail.



