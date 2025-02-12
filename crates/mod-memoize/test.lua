local kumo = require 'kumo'

local function compute_table(n)
  local result = {}
  for i = 1, n do
    table.insert(result, i)
  end
  return result
end

local cached_compute = kumo.memoize(compute_table, {
  name = 'test_mod_memoize',
  ttl = '5 minutes',
  capacity = 10,
})

local memoized = cached_compute(5)
print(memoized)
print(type(memoized))
print(kumo.serde.json_encode(memoized))
assert(#memoized == 5)
