# configure_resolver

```lua
kumo.dns.configure_resolver { PARAMS }
```

{{since('2023.08.22-4d895015')}}

By default, KumoMTA will parse the system resolver configuration and use that
to drive its internal caching [Hickory
DNS](https://github.com/hickory-dns/hickory-dns) resolver.

This function allows you to configure DNS resolving differently from
your system configuration.

!!! note
    This function should be called only from inside your
    [init](../events/init.md) event handler.

`PARAMS` is a lua table with the following fields:

* `name_servers` - required; a list of name servers. Each entry can be either a
  simple string of the form `"IP:PORT"` or can be a lua table that allows
  specifying the protocol that should be used.
* `domain` - optional; the local dns domain name to append to names.
  Note that MX resolution in KumoMTA always appends a trailing `.` to
  the names from the envelope addresses so this setting should be
  irrelevant for MX resolution.
* `search` - optional; list of additional search domains.
  Note that MX resolution in KumoMTA always appends a trailing `.` to
  the names from the envelope addresses so this setting should be
  irrelevant for MX resolution.
* `options` - a lua table listing out additional resolver options.
  The possible names, values and meanings are documented in
  the hickory DNS resolver documentation; see the table below:

|KumoMTA Version|Hickory DNS ResolverOpts|
|---------------|------------------------|
|2025.03.19-1d3f1f67|[hickory DNS 0.24](https://docs.rs/hickory-resolver/0.24.1/hickory_resolver/config/struct.ResolverOpts.html)|
|2025.05.06-b29689af|[hickory DNS 0.25](https://docs.rs/hickory-resolver/0.25.1/hickory_resolver/config/struct.ResolverOpts.html)|

```lua
kumo.on('init', function()
  kumo.dns.configure_resolver {
    name_servers = {
      -- Simple UDP based entry
      '10.0.0.1:53',
      -- Use tcp with a controlled local address
      {
        socket_addr = '10.0.0.20:53',
        protocol = 'tcp',
        -- an NXDOMAIN entry will be treated as truth and
        -- we won't query other nameservers to see if they
        -- can resolve a given query
        trust_negative_responses = true,
        bind_addr = '10.0.0.2:0',
      },
    },
    options = {
      edns0 = true,
      use_hosts_file = 'Auto',
    },
  }
end)
```

See also [kumo.dns.configure_unbound_resolver](configure_unbound_resolver.md).
