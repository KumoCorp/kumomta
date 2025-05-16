# kumo.string.psl_suffix

```lua
kumo.string.psl_suffix(STRING)
```

{{since('2024.09.02-c5476b89')}}

Get the public suffix as defined by Mozilla's [Public Suffix
List](https://publicsuffix.org/).

```lua
assert(kumo.string.psl_suffix 'www.example.com' == 'com')
```

