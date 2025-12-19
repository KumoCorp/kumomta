# define_resolver

```lua
kumo.dns.define_resolver(NAME, CONFIG)
```

{{since('2025.12.02-67ee9e96')}}

This function defines an alternative resolver from the default configured via
[configure_resolver](configure_resolver.md), and gives it a name.

The alternate resolver name can then be optionally passed as a parameter to a
number of other `kumo.dns` functions.

The intended use case is to define an alternate resolver that points to an
[rbldnsd](https://www.corpit.ru/mjt/rbldnsd.html) or similar specialized
resolver that provides access to a
[DNSBL](https://en.wikipedia.org/wiki/Domain_Name_System_blocklist).

The `NAME` parameter is a string that defines the name of the alternate resolver.

The `CONFIG` parameter defines the parameters for the resolver.  It can have
one of the following shapes:

!!! note
    This function should be called only from inside your
    [init](../events/init.md) event handler.

## Hickory with an explicit upstream

If you have `rbldnsd` or similar available on `10.0.0.1:53`, then you might use this:

```lua
kumo.dns.define_resolver('rbl', {
  Hickory = {
    name_servers = {
      '10.0.0.1:53',
    },
  },
})
```

You can then query it:

```lua
local answer, reason = kumo.dns.rbl_lookup(IP, 'rbl.domain', 'rbl')
```

## Test or static DNS

If you have fixed and locally available zone data, then you can query
that explicitly:

```lua
kumo.dns.define_resolver('rbl', {
  Test = {
    zones = {
      [[
$ORIGIN rbl.domain.
1.0.0.10 30 IN A   127.0.0.2
1.0.0.10 300   TXT "Blocked for a very good reason!"
  ]],
    },
  },
})
```
You can then query it:

```lua
local answer, reason = kumo.dns.rbl_lookup('10.0.0.1', 'rbl.domain', 'rbl')
```

This mode of operation was originally intended for testing, but may prove
useful in other situations.

## System Default

```lua
kumo.dns.define_resolver('myresolver', 'HickorySystemConfig')
```

Parses the system resolver configuration and applies that to a separate
instance of the hickory DNS resolver client. This is equivalent to the default
resolver settings in kumomta.

## Unbound with an explicit upstream

!!! note
    We generally recommend sticking with Hickory unless you have a very good
    reason.

If you have `rbldnsd` or similar available on `10.0.0.1:53`, then you might use this:

```lua
kumo.dns.define_resolver('rbl', {
  Unbound = {
    name_servers = {
      '10.0.0.1:53',
    },
  },
})
```

You can then query it:

```lua
local answer, reason = kumo.dns.rbl_lookup(IP, 'rbl.domain', 'rbl')
```

## Aggregating Different Resolvers

If you have a mixture of local zone files and a remote DNS, then you can
mix them together; have the local zones queried before falling back to
a remote host.

In the example below, the local zone is used first before falling back
to querying the upstream specified by the system.

```lua
kumo.dns.define_resolve('aggregate', {
  Aggregate = {
    -- The value of `Aggregate` here is an array style table
    -- listing out one of the CONFIG options shown in the
    -- examples above.

    -- First we have a Test setup
    Test = {
      zones = {
        [[
$ORIGIN 0.0.127.in-addr.arpa.
1 30 IN PTR localhost.
  ]],
      },
    },

    -- Then we have a system default setup
    'HickorySystemConfig',
  },
})
```

