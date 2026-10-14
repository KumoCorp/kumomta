# kumo.string.psl_domain_is_known

```lua
kumo.string.psl_domain_is_known(STRING)
```

{{since('TBD')}}

Check whether the registrable domain's public suffix is explicitly listed in
Mozilla's [Public Suffix
List](https://publicsuffix.org/).

Returns `true` when the input is a registrable domain whose public suffix is
explicitly listed, or `false` when the suffix was derived from an unlisted
label, or when the input has no registrable domain.

```lua
assert(kumo.string.psl_domain_is_known 'www.example.com' == true)
assert(kumo.string.psl_domain_is_known 'domain.invalid' == false)
assert(kumo.string.psl_domain_is_known 'com' == false)
```
