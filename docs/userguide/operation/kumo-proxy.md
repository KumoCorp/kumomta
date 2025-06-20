# Using the KumoProxy Socks5 proxy utility

KumoMTA comes with a socks5 proxy of our own design to assist with deployment of cluster environments.

The binary is located at `/opt/kumomta/sbin/proxy-server` and can be operated independently from KumoMTA.

The fast path to success is to clone the KumoMTA repo to a new server that has network capabilities and public facing IPs.  You do not need to configure or start KumoMTA.  To execute the proxy, use the command `/opt/kumomta/sbin/proxy-server --listen <IP:Port>`

Ensure that you have configured `sysctl` to allow for enough file handles, ip forwarding and other important factors. 

Usage documentation is at `/opt/kumomta/sbin/proxy-server --help`

## Configuring KumpProxy to survive a restart

KumoProxy can be configured as a systemd service

Note that this has only been tested for Ubuntu 24.  Your OS may require slightly different configuration.

You will need sudo access to perform these changes.  Start by creating a service file.

`sudo vi /etc/systemd/system/kumoproxy.service`

Populate it with:
```console
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

IE:
```console
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

