# kumo.string.wrap

```lua
local wrapped = kumo.string.wrap(TEXT, SOFT_LIMIT, HARD_LIMIT)
```

{{since('2025.12.02-67ee9e96')}}

Ensures that the `TEXT` parameter is no longer than `HARD_LIMIT` by breaking
around whitespace to try to keep most lines below `SOFT_LIMIT`, but hard
breaking longer words that exceed `HARD_LIMIT`.

Any breaks introduced will be `\r\n\t` (CR, LF, then TAB).

`SOFT_LIMIT` defaults to `75`.

`HARD_LIMIT` defaults to `900`.

Any trailing whitespace will be trimmed from the returned value.

## Example

```lua
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

utils.assert_eq(kumo.string.wrap 'hello', 'hello')

local long_string = string.rep('A', 200)
local expect_wrapped = string.rep('A', 100)
  .. '\r\n\t'
  .. string.rep('A', 100)
utils.assert_eq(kumo.string.wrap(long_string, 75, 100), expect_wrapped)

local long_string_spaced = string.rep('hello there ', 10)
utils.assert_eq(
  kumo.string.wrap(long_string_spaced, 75, 100),
  'hello there hello there hello there hello there hello there hello there\r\n'
    .. '\thello there hello there hello there hello there'
)
```
