# log_arf

Optional boolean. Defaults to `false`. When set to `true`, if the incoming mail
is an ARF feedback report formatted according to RFC 5965, and is addressed to
the requested domain, the message will be accepted and logged to the delivery
logs.

```lua
kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  if domain == 'fbl.example.com' then
    return kumo.make_listener_domain {
      log_arf = true,
    }
  end
end)
```


