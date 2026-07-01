# Using the KumoProxy Socks5 proxy utility

KumoMTA comes with a socks5 proxy of our own design to assist with deployment of cluster environments.

The binary is located at `/opt/kumomta/sbin/proxy-server` and can be operated independently from KumoMTA.

The fast path to success is to clone the KumoMTA repo to a new server that has network capabilities and public facing IPs.  You do not need to configure or start KumoMTA.  To execute the proxy, use the command `/opt/kumomta/sbin/proxy-server --listen <IP:Port>`

Ensure that you have configured `sysctl` to allow for enough file handles, ip forwarding and other important factors. 

Usage documentation is at `/opt/kumomta/sbin/proxy-server --help`

## Configuring KumpProxy to survive a restart

The above simplified instructions work well with the classic proxy-server cli, but as of the 2026-03-04 release there have been many enhancements.  The instructions below leverage those changes.

KumoProxy can be configured as a systemd service and can read configuration from a Lua file.

Note that the instructions here were tested on Ubuntu 24, your OS may require slightly different configuration.

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
ExecStart=/opt/kumomta/sbin/proxy-server --proxy-config /opt/kumomta/etc/policy/proxy_init.lua
TimeoutStopSec=10

[Install]
WantedBy=multi-user.target
```

Next, create an lua config file where you can add your proxy config.

`sudo vi /opt/kumomta/etc/policy/proxy_init.lua`

Populate with your proxy config.  You can read more about proxy config options [here](../../reference/proxy/start_proxy_listener/_index.md)

IE:
```lua
local kumo = require 'kumo'
local proxy = require 'proxy'

kumo.on('proxy_init', function()
  -- Start SOCKS5 proxy listener on port 5000 across all plumbed IPs
  proxy.start_proxy_listener {
    listen = '0.0.0.0:5000',
    timeout = '60 seconds',
  }

  -- Start HTTP listener for metrics and administration
  proxy.start_http_listener {
    listen = '0.0.0.0:8000',
    trusted_hosts = { '127.0.0.1', '::1' },
  }
end)
```

Reload the services:
`sudo systemctl daemon-reload`

Now you can `start`, test `status` and `enable` like any other service.  Do these manually once to ensure the service starts.  After it is `enabled` it will restart automatically on reboot.
```bash
sudo systemctl start kumoproxy
sudo systemctl status kumoproxy
sudo systemctl enable kumoproxy
```

