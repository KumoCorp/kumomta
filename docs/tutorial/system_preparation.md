# System Preparation

Now that we know **_what_** to build, lets go ahead and build it.

* Open up AWS, select EC2 and hit the [Launch Instance] button.
* Give this server a name and then search for "rocky" in the OS images.  When you have found it, select "Rocky 9".
* Under Instance Type, select a t2.xlarge, provide your keypair for login or create a pair if necessary.
* In the Network Settings, select or create a security group that includes ports 22,25,80,443,587,2025. These will be important for sending and receiving email in a number of ways.
* Finally, modify the storage volume to 300Gb (or anything over 100Gb) and click [Launch Instance].

... wait ....

When AWS has finished building your server instance, you can select it and connect. I prefer to find the SSH client information and use a remote terminal emulator like Putty or Terminal like this:

```console
ssh -i "yourkeyname.pem" rocky@ec2-\<pub-lic-ip\>.us-west-2.compute.amazonaws.com
```

## Doing the basics

Reguardless of what system you deploy, there are things you need to do to prepare the OS before installing the MTA.

* Update to the latest patches
* Install basic testing and support tools
* Turn off services that are wasteful or can interfere
* Tune the use of memory and file access for best performance
* Automate updates and startup for resilliency

### Rocky Linux Example

Rocky Linux is very similar to CentOS, as is Alma and RHEL  The instructions below are shown for a Rocky 9 system but with slight modification, should work for any DNF package management system. For Amazon Linux (AL2) the instructions are identical, but replace "dnf" with "yum".

```console
# Do basic updates
sudo dnf clean all
sudo dnf update -y

# Grab some handy tools
sudo dnf install -y wget bind bind-utils telnet firewalld ...
```

It is always a good idea to automate daily systems updates.

```console
# Make sure it all stays up to date
# Run a dnf update at 3AM daily
echo "0 3 * * * root /usr/bin/dnf update -y >/dev/null 2>&1" | sudo tee /etc/cron.d/dnf-updates >/dev/null
```

... and configure the local firewall...
```console
# Build a basic firewall
sudo echo "ZONE=public
" | sudo tee -a /etc/sysconfig/network-scripts/ifcfg-eth0

sudo systemctl stop firewalld
sudo systemctl start firewalld.service
sudo firewall-cmd --set-default-zone=public
sudo firewall-cmd --zone=public --change-interface=eth0
sudo firewall-cmd --zone=public --permanent --add-service=http
sudo firewall-cmd --zone=public --permanent --add-service=https
sudo firewall-cmd --zone=public --permanent --add-service=ssh
sudo firewall-cmd --zone=public --permanent --add-service=smtp
sudo firewall-cmd --zone=public --permanent --add-port=587/tcp

sudo systemctl enable firewalld
sudo firewall-cmd --reload
```

And finally, disabling unnecessary services like postfix and qpidd

```console
$ sudo systemctl stop  postfix.service
$ sudo systemctl disable postfix.service
$ sudo systemctl stop  qpidd.service
$ sudo systemctl disable qpidd.service
```

