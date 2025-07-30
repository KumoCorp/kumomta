# socks5_proxy_server

{{since('2023.06.22-51b72a83')}}

Optional string.

If both `socks5_proxy_server` and `socks5_proxy_source_address` are specified, then
SMTP connections will be made via a SOCKS5 Proxy server.

hostname support {{since('dev', inline=True)}}
`socks5_proxy_server` specifies the address and port of the proxy server. This can be specified as either:

* A SocketAddr (e.g., "127.0.0.1:5000" or "[::1]:5000")
* A hostname (e.g., "socks5.example.com:5000")

When a hostname is specified, it will be resolved to an IP address at runtime when the connection is established.

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1, via a proxy server
    kumo.make_egress_source {
      name = 'ip-1',
      socks5_proxy_source_address = '10.0.0.1',
      socks5_proxy_server = '10.0.0.1:5000',
      ehlo_domain = 'mta1.examplecorp.com',
    }
  end
  error 'you need to do something for other source names'
end)
```

Example with hostname:

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1, via a proxy server using hostname
    kumo.make_egress_source {
      name = 'ip-1',
      socks5_proxy_source_address = '10.0.0.1',
      socks5_proxy_server = 'socks5.example.com:5000',
      ehlo_domain = 'mta1.examplecorp.com',
    }
  end
  error 'you need to do something for other source names'
end)
```


