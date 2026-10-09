# kumo.add_charset_alias

```lua
kumo.add_charset_alias{ {'target', {'alias1', 'alias2'}}}
```

{{since('dev')}}

The same character set is often known by different names depending on the
operating system or application that produced a message. A message using a name
KumoMTA does not recognize fails to decode, even when the bytes are valid.

`kumo.add_charset_alias` registers additional names for a `target` character set
so those messages decode correctly. The `target` must be a character set KumoMTA
already recognizes. Aliases apply to both encoded-word headers and MIME bodies,
and should be registered from your [init](../events/init.md) event handler.

```lua
kumo.on('init', function()
  kumo.add_charset_alias{
    { 'euc-kr', { 'ms949', 'cp949' } },
  }
end)
```
