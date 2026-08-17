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

{{since('dev')}}

Configuration is now validated strictly against the kumomta resolver options
schema. The unbound backend honors only the option fields that have a
meaningful mapping to unbound's own configuration:

* `validate` — when `true`, the built-in DNSSEC trust anchors are loaded
  into the unbound context.
* `trust_anchor_file` — passed through to unbound's `load_trust_anchor_file`.
* `use_hosts_file` — `Always` or `Auto` loads `/etc/hosts`; `Never` skips it.

Any other `options` field set on a config passed to
`configure_unbound_resolver` is a configuration-time error with a message
identifying the offending field. If you need fields that are only meaningful
to hickory (such as `ndots`, `timeout`, `cache_size`, etc.) use
[kumo.dns.configure_resolver](configure_resolver.md) instead.

The `protocol` field on individual `name_servers` entries is accepted but
has no effect on the unbound backend, since unbound chooses UDP/TCP
internally per query.

See also [kumo.dns.configure_resolver](configure_resolver.md),
[kumo.dns.define_resolver](define_resolver.md), and
[kumo.dns.load_resolv_conf](load_resolv_conf.md).
