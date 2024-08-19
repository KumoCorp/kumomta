# ehlo_domain

Optional string.

If set, specifies the hostname to be passed with the EHLO command when the server connects to a remote host.

If not specified, the kernel will use the server's hostname.

Note that the `ehlo_domain` set by [make_egress_path](../make_egress_path/index.md), if any,
takes precedence over this value.

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1
    kumo.make_egress_source {
      name = 'ip-1',
      source_address = '10.0.0.1',
      ehlo_domain = 'mta1.examplecorp.com',
    }
  end
  error 'you need to do something for other source names'
end)
```


