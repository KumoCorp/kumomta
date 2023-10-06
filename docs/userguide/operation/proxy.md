# Routing Messages Via Proxy Servers

KumoMTA support SOCK5 and HAProxy for use as forward proxies as part of message delivery.

The most common use cases for a forward proxy are:

* Sharing IP addresses between multiple KumoMTA instances for high availability.
* Leveraging more IP addresses than permitted per instance by a hosting provider.
* Utilizing IP addresses in remote environments without additional MTA instances.

Due to the limitations inherent in the HAProxy protocol when used as a forward proxy, it is strongly recommended that SOCKS5 be utilized when possible, but HAProxy support is provided for existing installations migrating to KumoMTA.

## Using SOCKS5 for Delivery

### Using The KumoProxy SOCKS5 Proxy Server

While KumoMTA will work with any compliant SOCKS5 proxy server, we have built KumoProxy to serve as an integrated and supported proxy server specifically for use with KumoMTA.

*TODO** Install instructions are pending for the Beta 3 release.

### Configuring an egress_source for SOCKS5 Proxy Use

Configuring an egress_source to use a SOCKS5 proxy server is done as part of the `define_egress_source`
function call:

```lua
kumo.on('init', function()
  -- Make a source that will emit from 10.0.0.1, via a proxy server
  kumo.define_egress_source {
    name = 'ip-1',

    -- The SOCKS5 proxy server address and port
    socks5_proxy_server = '10.0.5.10:5000',

    -- Used by the SOCKS5 proxy server to connect to the destination address
    socks5_proxy_source_address = '10.0.0.1',

    ehlo_domain = 'mta1.examplecorp.com',
  }
end)
```

The SOCKS5 proxy server will forward communications via the
`socks5_proxy_source_address` IP address to reach the remote destination host.

Each IP address hosted by a SOCKS5 proxy server should be defined as its own
`egress_source`, IPv4 and IPv6 should be configured as separate sources, but
can be hosted by the same HAProxy instance(s).

See the [make_egress_source](../../reference/kumo/make_egress_source.md)
page of the Reference Manual for more information.

## Using HAProxy for Delivery

KumoMTA supports V2 of the [HAProxy](https://www.haproxy.org/) [PROXY
protocol](https://www.haproxy.org/download/2.7/doc/proxy-protocol.txt),
enabling the use of HAProxy as a forward SMTP proxy for the delivery of
messages via IP addresses on the HAProxy host.

### Configuring an egress_source for HAProxy Use

Configuring an egress_source to use an HAProxy server is done as part of the `define_egress_source`
function call:

```lua
kumo.on('init', function()
  -- Make a source that will emit from 10.0.0.1, via a proxy server
  kumo.define_egress_source {
    name = 'ip-1',

    -- The HAProxy server address and port
    ha_proxy_server = '10.0.5.10:5000',

    -- Used by HAProxy to connect to the destination address
    ha_proxy_source_address = '10.0.0.1',

    ehlo_domain = 'mta1.examplecorp.com',
  }
end)
```

The HAProxy server will forward communications via the
`ha_proxy_source_address` IP address to reach the remote destination host.

Each IP address hosted by an HAProxy instance should be defined as its own
`egress_source`, IPv4 and IPv6 should be configured as separate sources, but
can be hosted by the same HAProxy instance(s).

See the [make_egress_source](../../reference/kumo/make_egress_source.md)
page of the Reference Manual for more information.

### HAProxy Server Configuration

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
$ sudo haproxy -f haproxy.conf -V
```

For further information on HAProxy, see [the HAProxy Documentation](http://docs.haproxy.org/dev/intro.html).
