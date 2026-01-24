local kumo = require 'kumo'

kumo.set_memory_hard_limit(100000)
kumo.set_memory_soft_limit(80000)

local low_limit = kumo.get_memory_low_thresh()
local soft_limit = kumo.get_memory_soft_limit()
local hard_limit = kumo.get_memory_hard_limit()

assert(
  soft_limit == 80000,
  'Expected soft limit to be 80000, got ' .. tostring(soft_limit)
)
assert(
  hard_limit == 100000,
  'Expected hard limit to be 100000, got ' .. tostring(hard_limit)
)
assert(
  low_limit == 64000,
  'Expected low limit to be 64000 (80% of 80000), got ' .. tostring(low_limit)
)
