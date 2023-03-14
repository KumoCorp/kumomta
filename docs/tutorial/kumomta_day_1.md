# KumoMTA - Day 1

This document will outline a tpical deployment of KumoMTA starting from scratch.  We will assume you have nothing and need to get a functional production MTA set up by the end of the day.  With some planning and maybe a little luck we will get you there in time for lunch :)

This walkthrough is not a replacement for reading the full documentation, but rather will show how to install and configure in a specific environment as a sample that you can bend to your own needs.

We assume that you have an AWS account, Github account, and have a working knowledge of how those tools operate. We also assume that you know what an MTA is and why you need one.  If not, you may want to [read this first](https://en.wikipedia.org/wiki/Message_transfer_agent).

## Getting Started

**The scenario** we are going to emulate is a deployment using Rocky Linux V9 in AWS Public cloud. This will be a single node server having to send about eight million messages a day to the public Internet. The average size of these messages will be 50KB.

## Picking a Server

Considerations here have to balance performance with cost.  With the requirement of 8 million messages per day, we are also going to assume that is not evenly distributed.  In our scenario, we will assume those eight million messages will be sent roughly evenly over an 8 hour window (how convenient...).  

Lets do some math!

8 Million / 8 hours = 1 Million/hour

1 Million * 50KB = 50GB total volume/hour (400GB/day)

50GB / 3600s = 111 Mbps (0.1Gbps) Bandwidth

Knowing this we can plan the server size we need.  Sending 1 Million 50KB Messages per hour is actually pretty typical in the world of high performance MTAs so this sample build should suit a large portion of readers. 

### NETWORK
The bandwidth above is important because AWS EC2 network interfaces typically [support 5Gbps](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-instance-network-bandwidth.html) at the low end.  This means you dont need to stress about adding any network capacity for this build. 

### STORAGE VOLUME
The total volume is inportant for calculating storage capacity.  KumoMTA does not store the full body after delivery, but it will be needed to calculate spool capacity and memory use.  In a worst case scenario, if all of your messages are deferred (temporarily undeliverable) then your delayed queue is going to need at least this amount of space\* to store them.  In the example case, we may need to store 400GB for the full day's queue.  
In reality, most of that will drain in realtime and the better your sending reputation, the less spool storage you need, so there is one more good reason to maintain a good sending reputation. For the purposes of this sample, we are going to assume a 50% delivery rate so we will only need half that volume for spool.
\* Spool storage is calculated as ((Average_Message_Size + 50) x volume)

Logs will also consume drive space.  The amount of space is highly depended on your specific configuration, but based on the default sample policy, each log line will consume about 1KB and there may be an avarage of 5 log lines per message.  In this example we are going to assume logging at a rate of 5KB per mesage and then double it for safety. 

(5KB * 8Million) * 2 = 80Gb (per day of log storage)

You are also going to need about 8Gb for OS which brings our total storage space need to 8 + 80 + 200 ~= lets call it 300GB.

### CPU
While you could theoretically get by with 1 vCPU, email is actually pretty hard on the number crunching when you do it right.  Cryptographic signing and rate calculations are only a couple of the factors in planning CPU utilization. We recommend a minimum of 4vCPUs and you will see benefits to increasing that as your message processing volume increases.

### RAM
KumoMTA will process as many messages in RAM as possible, so more is better.  We recommend 16Gb RAM, but you will see benefits from adding more as your message processing volume increases.

So, from all of that we see a need for 4 vCPUs, 16Gb RAM, and 300Gb.   In AWS, that translates to a t2.xlarge.
 

## System Preparation
Now that we know what to build, lets build it.  Open up AWS, select EC2 and hit the [Launch Instance] button.

Give this server a name and then search for "rocky" in the OS images.  When you have found it, select "Rocky 9".

Under Instance Type, select a t2.xlarge, provide your keypair for login or create a pair if necessary.

In the Network Settings, select or create a security group that includes ports 22,25,80,443,587,2025. These will be important for sending and receiving email in a number of ways.

Finally, modify the storage volume to 300Gb (or anything over 100Gb) and click [Launch Instance].

... wait ....

When AWS has finished building your server instance, you can select it and connect. I prefer to find the SSH client information and use a remote terminal emulator like Putty or Terminal like this:

 ssh -i "yourkeyname.pem" rocky@ec2-<pub-lic-ip>.us-west-2.compute.amazonaws.com  

## Installing It!
OK, conratulations for making it this far. Now we can actually install some software.  This is probably the easiest part.

In another section of the documentation we show how to build from source and it takes a while.  Here we are going to take the easy path and install the [prebuilt binary from the repo](https://docs.kumomta.com/tutorial/getting_started/).

```console
sudo dnf -y install dnf-plugins-core
sudo dnf config-manager \
    --add-repo \
    https://openrepo.kumomta.com/files/kumomta-rocky.repo
sudo yum install kumomta-dev
```
This installs the KumoMTA daemon to /opt/kumomta/sbin/kumod

Yup, thats it.  Well, sort of.  

Technically KumoMTA is now installed, but it will need a configuration policy in order to do anything useful. We should also add some helper apps and clean up the OS in general for security reasons.  Lets take care of that first.

### OS cleanup and helpers
Anytime you install a server instance, you should update packages to the latest patches as a matter of good server management.  It is also a good idea to automate those updates nightly so the system stays current.

There are a number of helper apps I like to install but are completley optional.  These include telnet and curl for console level testing, tree for easier viewing of directory structure, and mlocate as a nice replacement for find. Bind and bind-utils are included to provide a fast local DNS cache.  You might choose unbound or any other fast local caching DNS server, but you should have one.

In the process below you will see that I also proactively disable Postfix and Qpidd just in case they happen to be installed by default.  These will interfere with KumoMTA and need to be disabled. It is entirely possible that these are not installed so you will see error messages like ```Failed to stop postfix.service``` and that is fine - ignore them.

```console
# Do basic updates 
sudo dnf clean all
sudo dnf update -y

# Grab some handy tools
sudo dnf install -y chrony wget bind bind-utils telnet curl mlocate unzip sudo cronie tree

sudo updatedb

# Slightly more optional handy tools for dev work
sudo dnf install -y make gcc firewalld sysstat

# Disable Postfix and Qpidd so they do not interfere
sudo systemctl stop  postfix.service
sudo systemctl disable postfix.service
sudo systemctl stop  qpidd.service
sudo systemctl disable qpidd.service
```

This next part needs to be done as root so do a ```sudo -s``` before you run the bits below and remember ```exit``` when done.
```console
# Make sure it all stays up to date
# Run a dnf update at 3AM daily
sudo echo "0 3 * * * root /usr/bin/dnf update -y >/dev/null 2>&1">/etc/cron.d/dnf-updates

# Tune sysctl setings. Note that these are suggestions, 
#  you should tune according to your specific build

echo "
vm.max_map_count = 768000
net.core.rmem_default = 32768
net.core.wmem_default = 32768
net.core.rmem_max = 262144
net.core.wmem_max = 262144
fs.file-max = 250000
net.ipv4.ip_local_port_range = 5000 63000
net.ipv4.tcp_tw_reuse = 1
kernel.shmmax = 68719476736
net.core.somaxconn = 1024
vm.nr_hugepages = 20
kernel.shmmni = 4096
" >> /etc/sysctl.conf

/sbin/sysctl -p /etc/sysctl.conf
```

If you have done all that correctly, you can do handy things like this:

```console
cd /opt
tree
```

### Writing Config Policy
The KumoMTA configuration is entirely written in [Lua](https://www.lua.org/home.html).  If you have not heard of Lua before, that is ok, you are not alone.  It is a powerful scripting language that is easy to read and code, but if very powerful.  It is used for custom scripts in Cisco security appliances, Roblox, World of Warcraft, and really awesome MTAs. You can read more about how we leverage Lua [here](https://docs.kumomta.com/tutorial/lua_resources/).

To save you from writing your own policy from scratch, you can just download our sample from [here](https://github.com/kumomta/kumomta/blob/main/simple_policy.lua).
On your server you can just ...

```console
wget https://github.com/kumomta/kumomta/blob/main/simple_policy.lua 
``` 

That will provide you with a basic and safe sending configuration that will allow you to move on to the testing step - we can examine the details later.


## Your First Email

## Tune for Performance

## Performnance Test

## Now What?



