# configure_unbound_resolver

```lua
kumo.dns.configure_unbound_resolver { PARAMS }
```

{{since('2023.11.28-b5252a41')}}

By default, KumoMTA will parse the system resolver configuration and use that
to drive its internal caching [Hickory
DNS](https://github.com/hickory-dns/hickory-dns) resolver.

This function allows you to configure DNS resolving differently from your
system configuration, and to use [Unbound embedded DNS
resolver](https://www.nlnetlabs.nl/projects/unbound/about/).

If you have enabled DANE for output SMTP then you must enable the unbound
resolver in order to be able to process DNSSEC correctly.

!!! note
    This function should be called only from inside your
    [init](../events/init.md) event handler.

The parameters to this functions are the same as those to
[kumo.dns.configure_resolver](configure_resolver.md).

```lua
kumo.on('init', function()
  kumo.dns.configure_unbound_resolver {
    options = {
      -- Enable DNSSEC
      validate = true,
    },
    -- By default, if you omit `name_servers`, unbound will
    -- resolve via the root resolvers.
    -- We strongly recommend deploying local caching nameservers
    -- and referencing them here:
    -- name_servers = { '1.1.1.1:53' },
  }
end)
```
