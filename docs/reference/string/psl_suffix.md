# `kumo.string.psl_suffix(STRING)`

{{since('dev')}}

Get the public suffix as defined by Mozilla's [Public Suffix
List](https://publicsuffix.org/).

```lua
assert(kumo.string.psl_suffix 'www.example.com' == 'com')
```

