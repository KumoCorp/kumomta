# kumo.start_esmtp_listener

```lua
kumo.start_esmtp_listener { PARAMS }
```

Configure and start ESMTP service.

This function should be called only from inside your [init](../../events/init.md)
event handler.

To listen on multiple IP/port combinations, simply call
`kump.start_esmtp_listener` multiple times with the appropriate parameters.

```lua
kumo.on('init', function()
  -- use the same settings for ports 25 and 2026, without repeating them all
  for _, port in ipairs { 25, 2026 } do
    kumo.start_esmtp_listener {
      listen = '0:' .. tostring(port),
      relay_hosts = { '0.0.0.0/0' },
    }
  end
end)
```

!!! note
    You can also use the
    [smtp_server_get_dynamic_parameters](../../events/smtp_server_get_dynamic_parameters.md)
    event to dynamically adjust listener parameters. You cannot bind
    new ports or IPs that way, but if you are using the "any" address
    such as `0.0.0.0` or `::`, you can dynamically refine the parameters
    for IP-based virtual service.

`PARAMS` is a lua table that can accept the keys listed below:


## ESMTP Listener Parameters { data-search-exclude }
