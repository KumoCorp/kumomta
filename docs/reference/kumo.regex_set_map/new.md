# `kumo.regex_set_map.new([{MAP}])`

{{since('2023.11.28-b5252a41')}}

Create a new *regex set map* from a set of key/value pairs.

A regex set can efficiently match a haystack string against a list of multiple
regular expressions in a single search operation.  The search operation on the
regex set knows which regular expression matched, and that is used here to map
to a more meaningful value.

Since the regex set needs to be compiled, the set is considered to be
*immutable*; it cannot have entries added after it has been created,
so you need to build up a lua table with the mapping to pass to `new`.

```lua
-- This example categorizes text into either 'hello', 'bye' results
local map = kumo.regex_set_map.new {
  hello = 'hello',
  bye = 'bye',
  later = 'bye',
  -- more complex patterns need to quote the map key:
  ['Good day'] = 'hello',
}

assert(map['hello there'] == 'hello')
assert(map['goodbye'] == 'bye')
assert(map['see you later'] == 'bye')
assert(map['not.set'] == nil)
```

