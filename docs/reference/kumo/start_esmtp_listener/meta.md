# meta

{{since('2025.05.06-b29689af')}}

Pre-define [connection metadata](../../connectionmeta.md) values.

The value of this option is a mapping of metadata key strings to
corresponding metadata values.  Each of these will be set in
the connection metadata object.

In this example, the key `hello` is assigned the value `there` when
the connection is opened:

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
  meta = {
    hello = 'there',
  },
}
```

`meta` can be combined with [via](via.md) and [peer](peer.md) to adjust the
metadata based on the local or remote address associated with the session,
respectively. In the following example the `trusted` metadata key is assigned
to `true` for connections from loopback or a lan address, but `false` otherwise:

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
  meta = {
    -- default is to assume untrusted
    trusted = false,
  },
  peer = {
    ['127.0.0.0/24'] = {
      meta = {
        trusted = true,
      },
    },
    ['192.168.1.0/24'] = {
      meta = {
        trusted = true,
      },
    },
  },
}
```

When `meta` is processed and applied to the effective listener configuration,
its values are overlaid and **merged** into the configuration, so in the
example above, the `trusted` value is applied to the metadata first, then the
matching peer block will replace the `trusted` key with the values it defines,
*preserving any other keys that might already be defined in the connection
metadata*.

The implication of this merging scheme, coupled with the lua language, is that
you cannot unset a metadata key using this syntax, because trying to explicitly
assign a value of `nil` is equivalent to not specifying a value.  You can
replace the value with some other non-nil value though.
