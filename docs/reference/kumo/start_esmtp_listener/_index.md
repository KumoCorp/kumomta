# `kumo.start_esmtp_listener {PARAMS}`

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

`PARAMS` is a lua table that can accept the keys listed below:

