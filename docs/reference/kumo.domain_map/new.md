# `kumo.domain_map.new([{MAP}])`

Create a new *domain map*, optionally seeded with an initial set of
key/value pairs.

A domain map is a dictionary type that allows resolving the value
associated with a domain name key, supporting wildcard domain keys
in the mapping.

For simple inputs, the mapping behaves as you might expect for a dictionary
type:

```lua
local dmap = kumo.domain_map.new()
dmap['foo'] = 'bar'
assert(dmap['foo'] == 'bar')
assert(dmap['not.set'] == nil)
```

you can define wildcard keys:

```lua
local dmap = kumo.domain_map.new()

dmap['*.example.com'] = 'wildcard'

-- An exact lookup for example.com won't match the wildcard
assert(dmap['example.com'] == nil)

-- but any nodes "below" that will match the wildcard entry:
assert(dmap['foo.example.com'] == 'wildcard')

-- Any explicitly added entries will take precedence
-- over the wildcard:
dmap['explicit.example.com'] = 'explicit'
assert(dmap['explicit.example.com'] == 'explicit')
```

You may seed an initial value from a pre-existing lua table:

```lua
local dmap = kumo.domain_map.new {
  ['*.woot.com'] = 123,
  ['example.com'] = 24,
}

-- and mutate the table after is has been constructed:
dmap['*.example.com'] = 42

assert(dmap['lemon.example.com'] == 42)
assert(dmap['example.com'] == 24)
assert(dmap['woot.com'] == nil)
assert(dmap['aa.woot.com'] == 123)
```
