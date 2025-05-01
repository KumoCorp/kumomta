# Linux Tuning for Performance

KumoMTA performance can be optimized by fine tuning system parameters. The settings below are examples only but have helped optimize test and development servers. This example represents a starting off point, you should research these and tune as needed for your own system.

KumoMTA makes heavy use of files, RAM, CPU and network resources. Setting these can be helpful as a default Linux install assumes the need to share resources with many applications, but we need to allow KumoMTA to use as much of the resource pool as possible.

## Tuning sysctl.conf

The following tuning parameters should be reviewed to ensure they are properly tuned for your workload.

These parameters should be added or updated in */etc/sysctl.conf*:

* vm.max_map_count
* net.core.rmem_default
* net.core.wmem_default
* net.core.rmem_max
* net.core.wmem_max
* fs.file-max
* net.ipv4.ip_local_port_range
* net.ipv4.tcp_tw_reuse
* kernel.shmmax
* net.core.somaxconn
* vm.nr_hugepages
* kernel.shmmni

This does not necessarily represent the full range of kernel tuning parameters you may need to adjust.

KumoMTA Sponsors should consult with the KumoMTA support team for assistance with performance tuning. More information on sponsoring KumoMTA can be found [here](https://kumomta.com/support).
