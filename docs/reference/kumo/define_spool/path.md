# path

Specifies the path to the directory into which the spool will be stored.

```lua
kumo.on('init', function()
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo-spool/data',
  }
end)
```



