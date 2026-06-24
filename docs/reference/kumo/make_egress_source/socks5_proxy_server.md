# socks5_proxy_server

{{since('2023.06.22-51b72a83')}}

Optional string.

If both `socks5_proxy_server` and `socks5_proxy_source_address` are specified, then
SMTP connections will be made via a SOCKS5 Proxy server.

`socks5_proxy_server` specifies the address and port of the proxy server.

{{since('dev')}}
Can be a DNS host name + port (`proxy.example.com:5000`) in addition
to an IP literal (`10.0.0.1:5000`, `[::1]:5000`). DNS names are resolved
at connection time; if resolution returns multiple addresses, each is
tried in turn until one connects successfully or the configured
`connect_timeout` elapses. In earlier versions, only IP literals were
supported.

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


