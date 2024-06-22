# `kumo.string.psl_domain(STRING)`

{{since('dev')}}

Get the registrable domain as defined by Mozilla's [Public Suffix
List](https://publicsuffix.org/).

```lua
assert(kumo.string.psl_domain 'www.example.com' == 'example.com')
```

