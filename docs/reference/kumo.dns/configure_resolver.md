# `kumo.dns.configure_resolver{PARAMS}`

{{since('dev')}}

By default, KumoMTA will parse the system resolver configuration and
use that to drive its internal caching DNS resolver.

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
  the [trust DNS resolver ResolverOpts
  documentation](https://docs.rs/trust-dns-resolver/0.22.0/trust_dns_resolver/config/struct.ResolverOpts.html)

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
      edns = true,
      use_hosts_file = true,
    },
  }
end)
```

