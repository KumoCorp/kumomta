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
  simple string of the form `"IP:PORT"`, which is equivalent to specifying the
  detailed form with `protocol = 'udp_then_tcp'` {{since('dev', inline=True)}}.
  In earlier versions this simple string form configures UDP only.
  The detailed lua table form allows specifying the protocol and other
  per-server settings; see [Name server entries](#name-server-entries) below.
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

|KumoMTA Version|Resolver options reference|
|---------------|------------------------|
|2025.03.19-1d3f1f67|[hickory DNS 0.24](https://docs.rs/hickory-resolver/0.24.1/hickory_resolver/config/struct.ResolverOpts.html)|
|2025.05.06-b29689af|[hickory DNS 0.25](https://docs.rs/hickory-resolver/0.25.1/hickory_resolver/config/struct.ResolverOpts.html)|
|{{since('dev', inline=True)}}|[KumoMTA-defined](resolver_options/index.md)|

{{since('dev')}}

KumoMTA now defines its own resolver options schema instead of forwarding
raw hickory-DNS option names directly. Existing valid configs continue to
parse unchanged — the schema is intentionally a near-superset of the
hickory 0.25 shape. The notable behavioral and validation changes are:

* Unknown fields in `options` (and elsewhere in the resolver config) are
  now a configuration-time error, surfacing typos that previously were
  silently ignored.
* The simple `'IP:PORT'` form for name servers configures UDP with TCP
  fallback by default. See the [name server entries](#name-server-entries)
  section below.
* The detailed name server form's `protocol` field accepts an additional
  value, `'udp_then_tcp'`, which is identical to the default behavior
  of the simple string form.
* The default value of `trust_negative_responses` is now `true`.
* `options.validate` (DNSSEC) is honored on both the Hickory and Unbound
  backends.

```lua
kumo.on('init', function()
  kumo.dns.configure_resolver {
    name_servers = {
      -- Simple entry (UDP with TCP fallback in dev builds, UDP only in
      -- earlier releases)
      '10.0.0.1:53',
      -- Detailed entry with explicit protocol and bind address
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

## Structured resolver configurations

In addition to the `name_servers`/`options` form above, `configure_resolver`
accepts the same structured resolver configurations as
[kumo.dns.define_resolver](define_resolver.md), namely the `Hickory`,
`HickorySystemConfig`, `Unbound`, `Test`, and `Aggregate` forms.

The `Test` form provides fixed, locally-available zone data and is primarily
intended for testing. {{since('dev', inline=True)}} each entry in its `zones`
list may be either a plain zone string (an *insecure*, non-DNSSEC zone) or a
table of the form `{ zone = "...", secure = true }` whose answers are reported
as DNSSEC-validated; this is required to exercise features that only trust
securely-resolved data, such as
[DANE](../kumo/make_egress_path/enable_dane.md). The `Test` form also accepts
an optional `servfail` list of owner names for which any lookup returns
`SERVFAIL`, which is useful for exercising failure-handling paths.

```lua
kumo.on('init', function()
  kumo.dns.configure_resolver {
    Test = {
      zones = {
        -- A DNSSEC-validated (secure) zone
        {
          zone = [[
$ORIGIN dane.example.
@ 3600 IN MX 10 mx.dane.example.
mx 3600 IN A 127.0.0.1
_25._tcp.mx 3600 IN TLSA 3 1 1 abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789
  ]],
          secure = true,
        },
        -- A plain string entry is treated as an insecure zone
        [[
$ORIGIN insecure.example.
@ 3600 IN A 127.0.0.2
  ]],
      },
      -- Any lookup for these names returns SERVFAIL
      servfail = {
        '_25._tcp.broken.example',
      },
    },
  }
end)
```

See [define_resolver](define_resolver.md) for details on each of the structured
forms.

## Name server entries

Each entry in `name_servers` is one of:

### Simple string form

The string is parsed as `IP:PORT`, which is equivalent to specifying the
detailed form with `protocol = 'udp_then_tcp'` {{since('dev', inline=True)}}.
In earlier versions this simple string form configures UDP only.

### Detailed table form

```lua
name_servers = {
  {
    socket_addr = '10.0.0.20:53',
    protocol = 'tcp',
    trust_negative_responses = true,
    bind_addr = '10.0.0.2:0',
  },
}
```

Fields:

* `socket_addr` (string, required) — `IP:PORT` of the upstream resolver.
* `protocol` (string, optional) — Which transport(s) to configure for this
  server. Accepted values are `'udp'` and `'tcp'`. An additional value
  `'udp_then_tcp'` configures both transports on the same server, allowing
  same-server TCP fallback for truncated UDP responses
  {{since('dev', inline=True)}}. `'udp_then_tcp'` is also the default when
  `protocol` is omitted; earlier versions default to `'udp'`.
* `trust_negative_responses` (bool, optional) — When `true`, an NXDOMAIN
  response from this server is accepted as truth and other servers in the
  list are not consulted. When `false`, negative responses are retried against
  other configured servers. Defaults to `true` {{since('dev', inline=True)}};
  earlier versions default to `false`.
* `bind_addr` (string, optional) — Local `IP:PORT` to bind outgoing queries
  to.

See also [kumo.dns.configure_unbound_resolver](configure_unbound_resolver.md),
[kumo.dns.define_resolver](define_resolver.md), and
[kumo.dns.load_resolv_conf](load_resolv_conf.md).
