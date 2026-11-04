local kumo = require 'kumo'

-- A cache read at file scope and again from a handler after it expires,
-- forcing the handler's read through the detached populate path.
local count = 0
local function compute()
  count = count + 1
  return count
end

local cached = kumo.memoize(compute, {
  name = 'user.detached_reload',
  ttl = '1s',
  capacity = 1,
  populate_timeout = '5s',
})

local first = cached()
assert(
  first == 1,
  'expected file-scope call to compute 1, got ' .. tostring(first)
)

kumo.on('main', function()
  -- Called from a handler, this lookup takes the detached populate path by
  -- default.
  kumo.sleep(1.5)
  local v = cached()
  assert(v ~= nil, 'expected the detached populate to produce a value')
  local again = cached()
  assert(again == v, 'expected the second call to hit the cache')
  print 'memoize detached reload test OK'
end)
