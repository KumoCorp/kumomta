# kumo.string.psl_domain

```lua
kumo.string.psl_domain(STRING)
```

{{since('2024.09.02-c5476b89')}}

Get the registrable domain as defined by Mozilla's [Public Suffix
List](https://publicsuffix.org/).

```lua
assert(kumo.string.psl_domain 'www.example.com' == 'example.com')
```

