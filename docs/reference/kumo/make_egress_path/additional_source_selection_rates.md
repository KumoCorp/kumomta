# additional_source_selection_rates

{{since('dev')}}

Specifies additional source selection constraints that cut across the
per-site-per-source scoping of the
[source_selection_rate](source_selection_rate.md) option.

This option can be used to help manage IP-warmup.  Please read the
[source_selection_rate](source_selection_rate.md) documentation to understand
how source selection rate limiting functions.

The value of this option is a map from the *rate limit name* to the desired rate limit,
and allows you to express multiple constraints.

For example, you could implement a overall selection limit, independent of the destination site, for a given named source like this:

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    additional_source_selection_rates = {
      [string.format('overall-source-limit-%s', source_name)] = '10,000/day,max_burst=1',
    },
  }
end)
```

!!! note
    If you are using the shaping helper, it will automatically populate entries
    in this map when you specify `provider_source_selection_rate` in a provider
    block in your `shaping.toml` and will cause that limit to apply to that source
    across all sites that map to that provider:

```toml
[provider."Office 365".sources."new-source"]
provider_source_selection_rate = "500/d,max_burst=1"
```
