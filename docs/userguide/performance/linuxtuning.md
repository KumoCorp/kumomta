# Linux Tuning for Performance

KumoMTA performance can be optimized by fine tuning system parameters. The settings below are examples only but have helped optimize test and development servers. This example represents a starting off point, you should research these and tune as needed for your own system.

KumoMTA makes heavy use of files, RAM, CPU and network resources. Setting these can be helpful as a default Linux install assumes the need to share resources with many applications, but we need to allow KumoMTA to use as much of the resource pool as possible.

## Tuning sysctl.conf

Tuning Linux is beyond the scope of this documentation, for a guide to tuning `sysctl.conf` see [https://wiki.archlinux.org/title/Sysctl](https://wiki.archlinux.org/title/Sysctl).

The following are designed to increase the number of file file descriptors and the size of the range of ports that can be leveraged. Further tuning may be required on your specific system.

```bash
fs.file-max = 250000
net.ipv4.ip_local_port_range = 5000 63000
net.ipv4.tcp_tw_reuse = 1
```

After editing, the changes can be implemented without a restart with the **sysctl -p** command.
