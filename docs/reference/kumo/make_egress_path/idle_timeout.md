# idle_timeout
How long a connection will remain open and idle, waiting to be
reused for another delivery attempt, before being closed.

The value is specified as a integer in seconds, or as a string using syntax
like `"2min"` for a two minute duration. The default is `60s`.


```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    idle_timeout = 60,
  }
end)
```


