# lookup_txt

```lua
kumo.dns.lookup_txt(DOMAIN, OPT_RESOLVER_NAME)
```

{{since('2024.09.02-c5476b89')}}

Resolve a TXT record for the requested `DOMAIN`.

Raises an error if the domain doesn't exist.

Returns a lua array-style table with the list of txt records returned from DNS.

```lua
assert(
  kumo.serde.json_encode(kumo.dns.lookup_txt 'gmail.com')
    == '["v=spf1 redirect=_spf.google.com","globalsign-smime-dv=CDYX+XFHUw2wml6/Gb8+59BsH31KzUr6c1l2BPvqKX8="]'
)
```

{{since('dev')}}

The `OPT_RESOLVER_NAME` is an optional string parameter that specifies the name
of a alternate resolver defined via [define_resolver](define_resolver.md).  You
can omit this parameter and the default resolver will be used.
