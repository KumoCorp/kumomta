# kumo.string.psl_suffix_is_known

```lua
kumo.string.psl_suffix_is_known(STRING)
```

{{since('TBD')}}

Check whether the public suffix is explicitly listed in Mozilla's [Public Suffix
List](https://publicsuffix.org/).

Returns `true` when the suffix is explicitly listed, or `false` when the suffix
was derived from an unlisted label, or when the input has no public suffix.

```lua
assert(kumo.string.psl_suffix_is_known 'www.example.com' == true)
assert(kumo.string.psl_suffix_is_known 'domain.invalid' == false)
```
