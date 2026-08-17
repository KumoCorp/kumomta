# load_resolv_conf

```lua
local config = kumo.dns.load_resolv_conf()
local config = kumo.dns.load_resolv_conf '/path/to/resolv.conf'
```

{{since('dev')}}

Reads a `/etc/resolv.conf`-format file and returns a mutable Lua table
matching the shape accepted by
[kumo.dns.configure_resolver](configure_resolver.md). The intended use is
to start from the system upstream nameserver list and layer your own
resolver options on top before configuring the resolver.

If no path is provided, `/etc/resolv.conf` is read.

```lua
kumo.on('init', function()
  local cfg = kumo.dns.load_resolv_conf()
  cfg.options.positive_min_ttl = '5min'
  cfg.options.positive_max_ttl = '1h'
  cfg.options.cache_size = 16384
  kumo.dns.configure_resolver(cfg)
end)
```

## What is loaded from the file

* `nameserver` lines become entries in `name_servers`, configured for UDP
  with TCP fallback.
* `domain` and `search` lines populate `domain` and `search`.
* The `options` directives `ndots`, `timeout`, `attempts`, and `edns0`
  populate the corresponding `options` fields.

Other resolv.conf options (`rotate`, `single-request`, `trust-ad`, etc.) are
not surfaced; the loader exposes only the subset that maps cleanly to the
kumomta resolver options schema.

## Comparison with `HickorySystemConfig`

`kumo.dns.define_resolver(name, 'HickorySystemConfig')` uses the platform's
native system-config resolution path (the registry on Windows, the
SystemConfiguration framework on macOS, `/etc/resolv.conf` on Linux/BSD).
It does not allow layering your own `options` on top.

`kumo.dns.load_resolv_conf()` always reads a resolv.conf-format file (so it is
most useful on Linux/BSD systems) and returns a value you can mutate before
passing into `configure_resolver` or `define_resolver`.

See also [kumo.dns.configure_resolver](configure_resolver.md) and
[kumo.dns.define_resolver](define_resolver.md).
