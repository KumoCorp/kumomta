# Linux Tuning for Performance

KumoMTA performance can be optimized by fine tuning system parameters. The settings below are examples only but have helped optimize test and development servers. This example represents a starting off point, you should research these and tune as needed for your own system.

KumoMTA makes heavy use of files, RAM, CPU and network resources. Setting these can be helpful as a default Linux install assumes the need to share resources with many applications, but we need to allow KumoMTA to use as much of the resource pool as possible.

## Tuning sysctl.conf

The following tuning parameters can help KumoMTA fully leverage its host server resources.

These parameters should be added or updated in */etc/sysctl.conf*:

```bash
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
```

After editing, the changes can be implemented without a restart with the **sysctl -p** command.