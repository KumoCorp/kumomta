# `kumo.on('get_site_config', function(domain, site_name))`

```admonish
This event handler is in flux and may change significantly
```

Not the final form of this API, but this is currently how
we retrieve configuration used when making outbound
connections

```lua
kumo.on('get_site_config', function(domain, site_name)
  return kumo.make_site_config {
    enable_tls = 'OpportunisticInsecure',
  }
end)
```

See also [kumo.make_site_config](../kumo/make_site_config.md).
