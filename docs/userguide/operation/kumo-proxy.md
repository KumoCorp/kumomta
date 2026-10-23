---
description: Run the KumoProxy SOCKS5 proxy server to share sending IPs across a KumoMTA cluster, including installing it as a systemd service to survive restarts.
---

# Using the KumoProxy SOCKS5 proxy utility

KumoMTA comes with its own SOCKS5 proxy server to assist with deployment of cluster environments.

The binary is located at `/opt/kumomta/sbin/proxy-server` and can be operated independently from KumoMTA.

To run the proxy on a dedicated server with public-facing IPs, install the KumoMTA package on that server; you do not need to configure or start KumoMTA itself.  To execute the proxy, use the command `/opt/kumomta/sbin/proxy-server --listen <IP:Port>`

Ensure that you have configured `sysctl` to allow for enough file handles, ip forwarding and other important factors. 

Usage documentation is at `/opt/kumomta/sbin/proxy-server --help`

## Configuring KumoProxy to survive a restart

KumoProxy can be configured as a systemd service

Note that this has only been tested for Ubuntu 24.  Your OS may require slightly different configuration.

You will need sudo access to perform these changes.  Start by creating a service file.

`sudo vi /etc/systemd/system/kumoproxy.service`

Populate it with:
```txt
[Unit]
Description=KumoMTA SOCKS5 Proxy service
After=syslog.target network.target

[Service]
Type=simple
Restart=always
EnvironmentFile=-/opt/kumomta/etc/kumoproxy.env
ExecStart=/opt/kumomta/sbin/proxy-server --listen ${PROXY_IP}:${PROXY_PORT}
TimeoutStopSec=10

[Install]
WantedBy=multi-user.target
```

Next, create an environment file where you can add your system variables.

`sudo vi /opt/kumomta/etc/kumoproxy.env`

Populate with your IP and Port

For example:
```txt
PROXY_IP="172.31.37.164"
PROXY_PORT="5000"
```

Reload the services:
`sudo systemctl daemon-reload`

Now you can `start`, test `status` and `enable` like any other service.  Do these manually once to ensure the service starts.  After it is `enabled` it will restart automatically on reboot.
```bash
sudo systemctl start kumoproxy
sudo systemctl status kumoproxy
sudo systemctl enable kumoproxy
```

