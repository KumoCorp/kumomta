# System Preparation

Regardless of what OS and hardware you select, there are some basic things you should do to prepare your system before install. While vetran system admins will probably have done much of this already as a standard course of building a server, it is worth noting these to save you some stress later.

* In the cloud service network settings or local security appliance, create a security group that includes the ports you require access to.

    Port 22: SSH will be required to at least your own IP and is used for installing and managingthe server.

    Port 25: SMTP is required for both outbound mail and inbound async mail like out-of-band bounces and fbl messages.

    Port 80: HTTP is used for the HTTP API, but may only be required for a select set of IPs.  It should not be public.

    Port 443: HTTPS is just the secure (TLS) version of Port 80 so the same access rules apply. 

    Port 587: SMTP for Submission is not required, but recommended for injection of mail

    Port 2025: Alternate for SMTP Submission, also not required, but can be a handy alternatice.

* Update to the latest patches
It is always good to start with a clean and current system.

In dnf managed systems (Rocky, Alma, Fedora, etc) use
```bash
sudo dnf clean all
sudo dnf update -y
```
In apt managed systems (Debain, Ubuntu, etc) use
```bash
sudo apt-get -y update
sudo apt-get -y upgrade
```

* Install basic testing and support tools like firewalld tree telnet git bind (or bind9) bind-utils (or bind9-utils)
* Turn off services that can interfere, particularly postfix and qpidd
```bash
systemctl disable postfix
systemctl stop postfix
systemctl disable qpidd
systemctl stop qpidd
```
* Tune the use of memory and file access for best performance. In the sysctl settings, boosting fs.file-max up to 65535 and also setting tcp_tw_reuse = 1 will help performance.  Make other adjustments as needed to make maximum ue of RAM, file, and network resources.

* Automate updates and startup for resiliency
IE:
```bash
# Make sure it all stays up to date
# Run a dnf update at 3AM daily
echo "0 3 * * * root /usr/bin/dnf update -y >/dev/null 2>&1" | \
 sudo tee /etc/cron.d/dnf-updates >/dev/null
```
* Adding your system certificate, or at least generating a self signed certificate can be helpful before you start.  If you dont, one will be generated based on the available system parameters and the settings may not be what you want.


Now that you have a nicely prepared system, you can move on to installing the MTA.

