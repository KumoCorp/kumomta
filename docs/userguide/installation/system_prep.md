# System Preparation

Regardless of what OS and hardware you select, there are some basic things you should do to prepare your system before installing KumoMTA. While veteran system admins will probably have done much of this already as a standard course of building a server, it is worth noting these to save you some stress later.

* In the cloud service network settings or local security appliance, create a security group that includes the ports you require access to.

    Port 22: SSH will be required to access the host operating system

    Port 25: SMTP is required for both outbound mail and inbound mail, including injections and bounce messages.

    Port 80: HTTP is used for the HTTP API, but should be restricted to authorized hosts.

    Port 443: HTTPS is the secure (TLS) version of Port 80 so the same access rules apply.

    Port 587: SMTP for Submission is not required, but recommended for inbound messages.

    Port 2025: Alternate for SMTP Submission, useful in environments that restrict port 25.

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

Note that installing a caching name server is absolutely critical when you are using a high performance mail engine.  Please do yourself a favour and install bind (or some other caching name server) and test it now.
```bash
sudo apt install bind9 -y
sudo systemctl start named
```

* Turn off services that can interfere, particularly postfix and qpidd

```bash
sudo systemctl disable postfix
sudo systemctl stop postfix
sudo systemctl disable qpidd
sudo systemctl stop qpidd
```

* Tune the use of memory and file access for best performance. In the sysctl settings, boosting fs.file-max up to 65535 and also setting tcp_tw_reuse = 1 will help performance.  Make other adjustments as needed to make maximum use of RAM, file, and network resources.

* Automate updates and startup for resiliency

```bash
# Make sure it all stays up to date
# Run a dnf update at 3AM daily
echo "0 3 * * * root /usr/bin/dnf update -y >/dev/null 2>&1" | \
 sudo tee /etc/cron.d/dnf-updates >/dev/null
```

* Adding your system certificate, or at least generating a self-signed certificate can be helpful before you start.  If you don't, one will be generated based on the available system parameters and the settings may not be what you want.

Now that you have a nicely prepared system, you can move on to installing the MTA.
