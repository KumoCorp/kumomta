# `kumo.dns.lookup_txt(DOMAIN)`

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
