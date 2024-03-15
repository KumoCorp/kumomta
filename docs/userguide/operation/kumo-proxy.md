# Using the KumoProxy Socks5 proxy utility

KumoMTA comes with a socks5 proxy of our own design to assist with deployment of cluster environments.

The binary is located at `/opt/kumomta/sbin/proxy-server` and can be operated independently from KumoMTA.

The fast path to success is to clone the KumoMTA repo to a new server that has network capabilities and public facing IPs.  You do not need to configure or start KumoMTA.  To execute the proxy, use the command `/opt/kumomta/sbin/proxy-server --listen <IP:Port>`

Usage documentation is at `/opt/kumomta/sbin/proxy-server --help`

