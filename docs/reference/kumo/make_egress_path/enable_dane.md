# enable_dane

{{since('2023.11.28-b5252a41')}}

When set to `true` (the default is `false`), then `TLSA` records will be
resolved securely to determine the destination site policy for TLS according
to [DANE](https://datatracker.ietf.org/doc/html/rfc7672).

If TLSA records are available, then the effective value of `enable_tls` will
be treated as though it were set to `"Required"` and the OpenSSL DANE implementation
will be used to verify the server certificate against the TLSA records found
in DNS.

Use of DANE also *requires* functioning DNSSEC in your DNS resolver; you
will need to configure the `libunbound` resolver to successfully use DANE:

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
