# relay_from

Optional CIDR list. Defaults to an empty list. If the connected client is from
an IP address that matches the CIDR list, and the sending domain matches the
requested domain, then relaying will be allowed.

```lua
kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  if domain == 'send.example.com' then
    return kumo.make_listener_domain {
      relay_from = { '10.0.0.0/24' },
    }
  end
end)
```


