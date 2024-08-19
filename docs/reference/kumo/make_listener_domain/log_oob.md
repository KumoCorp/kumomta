# log_oob

Optional boolean. Defaults to `false`. When set to `true`, if the incoming mail
is an out-of-band (OOB) bounce report formatted according to RFC 3464, and is
addressed to the requested domain, the message will be accepted and logged to
the delivery logs.

```lua
kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  if domain == 'bounce.example.com' then
    return kumo.make_listener_domain {
      log_oob = true,
    }
  end
end)
```


