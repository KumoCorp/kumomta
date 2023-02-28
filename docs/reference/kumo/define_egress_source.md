# `kumo.define_egress_source {PARAMS}`

Defines an *egress source*, which is an entity associated with the source of
outbound traffic from the MTA.  A source must be referenced by a
[pool](define_egress_pool.md) to be useful.

This function should be called only from inside your [init](../events/init.md)
event handler.

A source must have at a minimum a *name*, which will be used in logging/reporting.

`PARAMS` is a lua table which may have the following keys:

## name

Required string.

The name of the source. If you call `kumo.define_egress_source` multiple
times with the same name, the most recently defined version of that name will replace
any previously defined source with that name.

```lua
kumo.on('init', function()
  -- Make a source that just has a name, but otherwise doesn't
  -- specify any particular source configuration
  kumo.define_egress_source {
    name = 'example',
  }
end)
```

## source_address

Optional string.

If set, specifies the local IP address that should be used as the source of any
connection that will be made from this source.

In not specified, the kernel will select the IP address automatically.

```lua
kumo.on('init', function()
  -- Make a source that will emit from 10.0.0.1
  kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.0.1',
  }
end)
```

## remote_port

Optional integer.

If set, will override the remote SMTP port number. This is useful in scenarios
where your network is set to manage the egress address based on port mapping.

This option takes precedence over
[kumo.make_egress_path().smtp_port](make_egress_path.md#smtp_port).

