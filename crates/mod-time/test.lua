local kumo = require 'kumo'

local now = kumo.time.now()
kumo.time.sleep(0.1)
local diff = now:elapsed()
-- convert to millisec
diff = diff / 1000000
assert(
  diff > 100 and diff < 110,
  'expected time difference between 100 ~ 110, got ' .. diff
)
