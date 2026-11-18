local kumo = require 'kumo'

-- A kumo.memoize cache called at file scope while kumod is still loading
-- this script must populate inline, not detached.

local counter = 0
local function compute()
  counter = counter + 1
  return counter
end

local cached = kumo.memoize(compute, {
  name = 'user.policy_load_regression',
  ttl = '1h',
  capacity = 1,
  populate_timeout = '5s',
})

-- Called at file scope, during the load.
local value = cached()
assert(
  value == 1,
  'expected inline populate to return 1, got ' .. tostring(value)
)

-- An explicit detached = true cannot populate inline without contradicting
-- the request, and is rejected with an error when reached during the load.
local detached = kumo.memoize(compute, {
  name = 'user.policy_load_detached',
  ttl = '1h',
  capacity = 1,
  detached = true,
})
local ok, err = pcall(detached)
err = tostring(err)
assert(not ok, 'expected detached = true call during load to error')
assert(
  string.find(
    err,
    'being called at file scope while the policy is still loading',
    1,
    true
  ),
  'unexpected error: ' .. err
)

kumo.on('main', function()
  print 'memoize policy-load regression test OK'
end)
