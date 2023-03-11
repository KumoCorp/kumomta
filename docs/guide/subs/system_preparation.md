# System Preparation

## Picking the right server size
Whether you install on bare metal or in a cloud, you will need a minimum of 4Gb RAM, 2 cores and 20Gb Storage.  While it may be possible to use a smaller container for the binary only, you will run into issues with the spool after only a few messages.  You can read more detail on this sizing in the page covering [KumoMTA Environmental Considerations](https://github.com/kumomta/kumomta/blob/main/docs/guide/subs/environment_consideration.md#kumomta-environmental-considerations)

A good sized instance for testing features would be 4 cores, 16Gb RAM, 100Gb Storage.  This is the build used for most of the testing shown in this document outside of the performance chart.  In AWS this is an m3.xlarge. In Azure, this is a B4ms.  In GCP, this is an e2-standard-4.

## Doing the basics

Reguardless of what system you deploy, there are things you need to do to prepare the OS.

- Update to the latest patches
- Install basic testing and support tools
- Turn off services that are wasteful or can interfere
- Tune the use of memory and file access for best performance
- Automate updates and startup for resilliency

### Rocky Linux Example

Rocky Linux is very similar to CentOS, as is Alma and RHEL  The instructions below are shown for a Rocky 8 system but with slight modification, should work for any DNF package management system.

```bash
# Do basic updates 
sudo dnf clean all
sudo dnf update -y

# Grab some handy tools
sudo dnf install -y chrony wget bind bind-utils telnet curl mlocate unzip sudo cronie

sudo systemctl enable chrony

# Slightly more optional handy tools for dev work
sudo dnf install -y make gcc firewalld sysstat
```

**These next 2 require actually being root so you need to manually set sudo, then run the following commands**

```sudo -s```

Then run these:
```
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

**Now exit from root user**

```exit```


The instructions below are shown for an Ubuntu 22 system but with slight modification, should work for any APT package management system.

```bash
# Do basic updates 
sudo apt-get -y update
sudo apt-get -y upgrade

# Grab some handy tools
sudo apt-get install -y chrony wget bind9 bind9-utils telnet curl mlocate unzip sudo cron

sudo systemctl enable chrony

# Slightly more optional handy tools for dev work
sudo apt-get install -y make gcc firewalld sysstat
```

```admonish
The following commands must be executed as the root user
```

```bash
# RUN AS ROOT
sudo -s

# Make sure it all stays up to date
# Run a dnf update at 3AM daily
sudo echo "0 3 * * * root /usr/bin/apt-get update -y >/dev/null 2>&1">/etc/cron.d/apt-get-updates
sudo echo "5 3 * * * root /usr/bin/apt-get upgrade -y >/dev/null 2>&1">>/etc/cron.d/apt-get-updates

# Tune sysctl setings. Note that these are suggestions, you should tune according to your specific build

sudo echo "
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


**Now exit from root user**

```exit```



## OS Hardening
Above the basics of any system deloyment, you may also want to do some "hardening".  This is the process of minimizing exposure to threats.  This is not a comprehensive list, but are some of the common things you should do to protect your system.

 - Disabling unnecessary services
   - postfix
   
```
sudo systemctl stop  postfix.service
sudo systemctl disable postfix.service
```

   - qpidd
   ```
sudo systemctl stop  qpidd.service
sudo systemctl disable qpidd.service
```

 - Firewall
 - SSH config
 - Switch to keypair only
 - 

 # TBD

Beyond the basics of any system deloyment, you may also want to do some "hardening".  This is the process of minimizing exposure to threats.  This is not a comprehensive list, but are some of the common things you should do to protect your system.

- Disabling unnecessary services
- Firewall
- SSH config
- Switch to keypair only
