# KumoMTA - Day 1

This document will outline a typical deployment of KumoMTA starting from scratch.  We will assume you have nothing and need to get a functional production MTA set up by the end of the day.  With some planning and maybe a little luck we will get you there in time for lunch :)

This walkthrough is not a replacement for reading the full documentation, but rather will show how to install and configure in a specific environment as a sample that you can bend to your own needs.

We assume that you have an AWS account, Github account, and have a working knowledge of how those tools operate. We also assume that you know what an MTA is and why you need one.  If not, you may want to [read this first](https://en.wikipedia.org/wiki/Message_transfer_agent).

## Getting Started
The scenario we are going to emulate is a deployment using Rocky Linux V9 in AWS Public cloud. This will be a single node server having to send about eight million messages a day to the public Internet. The average size of these messages will be 50KB.

## The TL;DR version
If you just want to get this installed and running, here are the quick steps with no explanation. This assumes you know what you are doing and just want the high-level info.  The longer version with deeper explanation follows in the next section.

1) Spin up an AWS t2.xlarge instance (or any server with 4vCPUs, 16Gb RAM, 300Gb Hard Drive)
2) Install Rocky linux 9
3) Update the OS and disable PostFix if needed

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

5) Create a configuration policy in ```/opt/kumomta/etc/policy/``` based on the example at [ https://docs.kumomta.com/userguide/configuration/example/](https://docs.kumomta.com/userguide/configuration/example/)
Hint, you can copy and paste that into a new file and edit the necessary parts.

6) Run it with (assuming you named your policy "example.lua") 
```console
sudo /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/example.lua --user rocky
```

And you are done.  KumoMTA will now be installed and running the example configuration from ```/opt/kumomta/sbin/kumod```.  If you want to dive into some details about WHY that all works, read on.


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
The total volume is inportant for calculating storage capacity.  KumoMTA does not store the full body after delivery, but it will be needed to calculate spool capacity and memory use.  In a worst case scenario, if all of your messages are deferred (temporarily undeliverable) then your delayed queue is going to need to store ALL them.  In the example case, we may need to store 400GB\* for the full day's queue.  

In reality, most of that will drain in realtime and the better your sending reputation, the less spool storage you need, so there is one more good reason to maintain a good sending reputation. For the purposes of this sample, we are going to assume a 50% delivery rate so we will only need half that volume for spool.

\* _Spool storage is calculated as ((Average_Message_Size_in_bytes + 512) x volume)_

Logs will also consume drive space.  The amount of space is highly dependent on your specific configuration, but based on the default sample policy, each log line will consume about 1KB and there may be an avarage of 5 log lines per message.  In this example we are going to assume logging at a rate of 5KB per mesage and then double it for safety. _We can talk more about reducing log space requirements ina later section._

(5KB * 8Million) * 2 = 80Gb (per day of log storage)

You are also going to need about 8Gb for OS which brings our total storage space need to 8 + 80 + 200 ~= lets call it 300GB.

### CPU
While you could theoretically get by with 1 vCPU, email is actually pretty hard on the number crunching when you do it right.  Cryptographic signing and rate calculations are only a couple of the factors in planning CPU utilization. We recommend a minimum of 4vCPUs and you will see benefits to increasing that as your message processing volume increases.

### RAM
KumoMTA will process as many messages in RAM as possible, so more is better.  We recommend 16Gb RAM, but you will see benefits from adding more as your message processing volume increases.

So, from all of that we see a need for 4 vCPUs, 16Gb RAM, and 300Gb.   In AWS, that translates to a t2.xlarge.


## System Preparation
Now that we know what to build, lets build it.  Open up AWS, select EC2 and hit the `Launch Instance` button.

Give this server a name and then search for "rocky" in the OS images.  When you have found it, select "Rocky 9".

Under Instance Type, select a t2.xlarge, provide your keypair for login or create a pair if necessary.

In the Network Settings, select or create a security group that includes ports 22,25,80,443,587,2025. These will be important for sending and receiving email in a number of ways.

Now that we know *_what_* to build, lets go ahead and build it.  
 - Open up AWS, select EC2 and hit the [Launch Instance] button.
 - Give this server a name and then search for "rocky" in the OS images.  When you have found it, select "Rocky 9".
 - Under Instance Type, select a t2.xlarge, provide your keypair for login or create a pair if necessary.
 - In the Network Settings, select or create a security group that includes ports 22,25,80,443,587,2025. These will be important for sending and receiving email in a number of ways.
 - Finally, modify the storage volume to 300Gb (or anything over 100Gb) and click [Launch Instance].

... wait ....

When AWS has finished building your server instance, you can select it and connect. I prefer to find the SSH client information and use a remote terminal emulator like Putty or Terminal like this:

```     ssh -i "yourkeyname.pem" rocky@ec2-\<pub-lic-ip\>.us-west-2.compute.amazonaws.com  ```

## Installing It!
Congratulations for making it this far. Now we can actually install some software.  This is probably the easiest part.

In another section of the documentation we show how to build from source and that can take a while.  Here, we are going to take the easy path and install the [prebuilt binary from the repo](https://docs.kumomta.com/tutorial/getting_started/).  You can literally just copy/paste the commands below to install it from our yum repo.

```console
$ sudo dnf -y install dnf-plugins-core
$ sudo dnf config-manager \
    --add-repo \
    https://openrepo.kumomta.com/files/kumomta-rocky.repo
$ sudo yum install kumomta-dev
```
This installs the KumoMTA daemon to /opt/kumomta/sbin/kumod

Done.

Yup, thats it.  

Well, sort of.  

Technically KumoMTA is now installed, but it will need a configuration policy in order to do anything useful. We should also add some helper apps and clean up the OS in general for security reasons.  Lets take care of that first.

### OS cleanup and helpers
Anytime you install a server instance, you should update packages to the latest patches as a matter of good server management.  It is also a good idea to automate those updates nightly so the system stays current.

There are a number of helper apps I like to install but are completley optional.  These include ```telnet``` and ```curl``` for console level testing, ```tree``` for easier viewing of directory structure, and ```mlocate``` as a nice replacement for find. ```Bind``` and ```bind-utils``` are included to provide a fast local DNS cache.  You might choose ```unbound``` or any other fast local caching DNS server, but you should have one.

In the process below you will see that I also proactively disable Postfix and Qpidd just in case they happen to be installed by default.  These will interfere with KumoMTA and need to be disabled. It is entirely possible that these are not installed so you might see error messages like ```Failed to stop postfix.service``` and that is fine - ignore them.

You should be able to just copy and paste this secion as-is.

```console
# Do basic updates
$ sudo dnf clean all
$ sudo dnf update -y

# Grab some handy tools
$ sudo dnf install -y chrony wget bind bind-utils telnet curl mlocate unzip sudo cronie tree

$ sudo updatedb

# Slightly more optional handy tools for dev work
$ sudo dnf install -y make gcc firewalld sysstat

# Disable Postfix and Qpidd so they do not interfere
$ sudo systemctl stop  postfix.service
$ sudo systemctl disable postfix.service
$ sudo systemctl stop  qpidd.service
$ sudo systemctl disable qpidd.service
```

If you have done all that correctly, you can do handy things like this:

```console
$ cd /opt
$ tree
```

### Writing Config Policy
The KumoMTA configuration is entirely written in [Lua](https://www.lua.org/home.html).  If you have not heard of Lua before, that is ok, you are not alone.  It is a powerful scripting language that is easy to read and code, but is very powerful.  It is used for custom scripts in Cisco security appliances, Roblox, World of Warcraft, and really awesome MTAs. You can read more about how we leverage Lua [here](https://docs.kumomta.com/tutorial/lua_resources/).

To save you from writing your own policy from scratch, you can just download our sample from [here](https://github.com/kumomta/kumomta/blob/main/simple_policy.lua). On your server you can just ...

```console
cd /opt/kumomta/etc/policy/
wget https://github.com/kumomta/kumomta/blob/main/simple_policy.lua 
``` 

That will provide you with a basic and safe sending configuration that will allow you to move on to the testing step - we can examine the details later.


## Your First Email
If you followed all the instructions above without errors, you shoudl now have a working MTA on a properly sized server.  Lets test that theory.

Start the MTA with this:
```console
 sudo /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/simple_policy.lua --user rocky &

```
 - Using sudo allows it to run as a privileged user so it can access port 25 which is needed to deliver via SMTP to the internet.
 - The daemon `kumod` is the MTA 
 - The directive --policy makes kumod load the `simple_policy.lua` file as configuration policy.  
 - Because we launched with sudo, you need to use the directive --user and provide a valid user to assign responsibility to.
 - The line ends with a `&` that forces the daemon to run in the background and returns you to a usable prompt (use `fg` to bring it back to the foreground)

You can test with a simple SMTP message right from the command line. The simple_policy defines a Listener on port 2025, so you can use that to inject a message.

```console
 telnet localhost 2025 
```

Now you can execute the following lines after changing the email addresses to your own.

```console
ehlo moto
mail from:youremail@address.com
rcpt to:youremail@address.com
DATA
from:youremail@address.com
to:youremail@address.com
subject: My First Email

Hey, this is my first email!

.

```

Check your mail to make sure it delivered.  

Note that if you have not [specifically requested outbound use of port 25](https://aws.amazon.com/premiumsupport/knowledge-center/ec2-port-25-throttle/) from AWS, then it is very possible the message will not be delivered.  If that is the case, try changing the outboud port to 465, which can sometimes be effective for low volume testing.

You can change the outbound port from the default 25 to 465 by editing the `remote_port` in the _egress_source_ definition like this: 

```console
kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.0.1',
    **remote_port - 465**
  }
end)
```

You will need to restart the daemon after that change in order for it to take effect.

Type ```fg``` to bring kumod to the forground, then CTRL-C to end the process.  After it stops, restart it with the normal start command.

```console
 sudo /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/simple_policy.lua --user rocky &
```

## Check the logs
Reguardless of whether the mail delivers or not, you should take a look at the logs.  Standard logs are found in ```/var/tmp/kumo-logs``` as can be seen in this tree. Logs are bundled by day and compressed so to read these, you need to unpack them first. The logs are typically stored in date formatted files, but you have [many options](https://docs.kumomta.com/userguide/configuration/logging/) and KumoMTA is highly configurable.

```info
/var/tmp/kumo-logs
├── 20230311-033705
├── 20230311-033844
├── 20230312-182542
└── 20230314-181435
```

We can take a look at a specific log by decompressing it first.




## Tune for Performance
This next part needs to be done as root so do a ``` sudo -s ``` before you run the bits below and remember ``` exit ``` when done.


You can save your self some hassle by automating patch updates with cron.
```console
# Make sure it all stays up to date
# Run a dnf update at 3AM daily
$ echo "0 3 * * * root /usr/bin/dnf update -y >/dev/null 2>&1" | sudo tee /etc/cron.d/dnf-updates >/dev/null
'''

You can get better performace with some fine tuning of system parameters,  The settings below are examples only but have worked in test and development servers.  As the saying goes, "your milage may vary" so you should research these and tune as needed for your own system.
```console
# Tune sysctl setings. Note that these are suggestions,
#  you should tune according to your specific build

$ echo "
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
" | sudo tee -a /etc/sysctl.conf > /dev/null

$ sudo /sbin/sysctl -p /etc/sysctl.conf
```

## Performnance Test
OK, now lets really test this with some volume.  You will not want to do that in the public internet with real adresses for a number of reasons, so you shoudl set up another KumoMTA instance and have it run the included "sink.lua" policy.  That will set KumoMTA to accept all messages and discard them without forwarding.

## Now What?

RTFM.  Seriously.  KumoMTA is a very powerful, highly configurable MTA that you can integrate in many ways.  There is no way we can document every possible use case or configuration.



