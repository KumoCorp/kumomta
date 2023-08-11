# `kumo.cidr.make_map([{MAP}])`

Create a new *CIDR map*, optionally seeded with an initial set of
key/value pairs.

A CIDR map is a dictionary type that allows resolving the value
associated with an IP address key, supporting matches based on CIDR blocks
in the mapping.

For simple inputs, the mapping behaves as you might expect for a dictionary
type:

```lua
local cmap = kumo.cidr.make_map()
cmap['127.0.0.1'] = 'bar'
assert(cmap['127.0.0.1'] == 'bar')
assert(cmap['not.set'] == nil)
```

you can define keys based on net blocks using CIDR notation:

```lua
local cmap = kumo.cidr.make_map()

cmap['10.0.0.0/24'] = '10. block'

-- any address in that /24 will return the mapped value
assert(cmap['10.0.0.1'] == '10. block')
assert(cmap['10.0.0.42'] == '10. block')

-- other addresses won't
assert(cmap['100.0.0.100'] == nil)
```

You may seed an initial value from a pre-existing lua table:

```lua
local cmap = kumo.cidr.make_map {
  ['127.0.0.0/24'] = 'loopback',
  ['10.0.0.0/24'] = 'lan',
}

-- and mutate the table after is has been constructed:
cmap['4.2.4.2'] = 4242
```

Keys to the map are IPv4 or IPv6 addresses, but for convenience, domain
literals and IP and port number combinations such as `"127.0.0.1:25"`,
`"[127.0.0.1]"` `"[::1]:25"` are understood to facilitate more ergonomic use in
policy:

```lua
local SOURCE_CLASSIFICATION = kumo.cidr.make_map {
  ['127.0.0.0/24'] = 'loopback',
  ['10.0.0.0/24'] = 'lan',
}

kumo.on('smtp_server_message_received', function(msg)
  local source_type = SOURCE_CLASSIFICATION[msg:get_meta 'received_from']
end)
```
