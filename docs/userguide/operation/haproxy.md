# Using HAProxy for Delivery

KumoMTA supports V2 of the [HAProxy](https://www.haproxy.org/) [PROXY protocol](https://www.haproxy.org/download/2.7/doc/proxy-protocol.txt), enabling the use of HAProxy as a forward SMTP proxy for the delivery of messages via IP addresses on the HAProxy host.

The most common use cases for HAProxy as a forward proxy are:

* Sharing IP addresses between multiple KumoMTA instances for high availability.
* Leveraging more IP addresses than permitted per instance by a hosting provider.
* Utilizing IP addresses in remote environments without additional MTA instances.

## Configuring an egress_source for HAProxy Use

Configuration of an HAProxy server is part of the define_egress_source function:

```lua
kumo.on('init', function()
  -- Make a source that will emit from 10.0.0.1, via a proxy server
  kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.5.1',
    ha_proxy_source_address = '10.0.0.1',
    ha_proxy_server = '10.0.5.10:5000',
    ehlo_domain = 'mta1.examplecorp.com',
  }
end)
```

The source_address option in the preceding example specifies what IP address the KumoMTA server should use for its outbound communications to the HAProxy server, which is defined by IP and port number in the `ha_proxy_server` option.

The HAProxy server will forward communications via the `ha_proxy_source_address` IP address to reach the remote destination host.

Each IP address hosted by an HAProxy instance should be defined as its own `egress_source`, IPv4 and IPv6 should be configured separately, but can be hosted by the same HAProxy instance(s).

See the [define_egress_source](../../reference/kumo/define_egress_source.md) page of the Reference Manual for more information.

## HAProxy Server Configuration

An example HAProxy server config is as follows:

```
global
    log stdout  format raw  local0  debug

defaults
    timeout connect 10s
    timeout client 30s
    timeout server 30s
    log global

listen outboundsmtp
    log global
    bind 0:2526 accept-proxy
    mode tcp
    use-server v4 if { src 0.0.0.0/0 }
    use-server v6 if { src ::/0 }
    server v4 0.0.0.0 source 0.0.0.0 usesrc clientip
    server v6 ::: source ::: usesrc clientip
```

The HAProxy instance would be launched with the following command:

```console
$ sudo haproxy -f assets/haproxy.conf -V
```

For further information on HAProxy, see [the HAProxy Documentation](http://docs.haproxy.org/dev/intro.html).
