local kumo = require 'kumo'

local ms = kumo.time.epoch_millis()
kumo.time.sleep(0.1)
local diff = kumo.time.since_millis(ms)
assert(
  diff > 100 and diff < 110,
  'expected time difference between 100 ~ 110, got ' .. diff
)
