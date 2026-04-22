local kumo = require 'kumo'

-- Confirm that initial value is set correctly
local initial = kumo.counter_series.define {
  name = 'test.initial',
  num_buckets = 6,
  bucket_size = '60s',
  initial_value = 10,
}

assert(initial:sum() == 10, 'sum after initial value should be 10')

local series = kumo.counter_series.define {
  name = 'test.series',
  num_buckets = 6,
  bucket_size = '1m',
}

-- Calling define with the same name and shape returns the cached series
-- and ignores initial_value.
local same = kumo.counter_series.define {
  name = 'test.series',
  num_buckets = 6,
  bucket_size = '60s',
  initial_value = 999,
}

series:increment(2)
series:delta(-1)
assert(series:sum() == 1, 'sum after increment+delta should be 1')
assert(same:sum() == 1, 'second handle should see same cached value')

series:observe(10)

assert(series:sum() == 10, 'sum after observe should be 10')
assert(
  series:sum_over '2m' == 10,
  'sum_over should include current bucket value'
)

-- Re-defining with a different shape replaces the series with a fresh one.
local replaced = kumo.counter_series.define {
  name = 'test.series',
  num_buckets = 4,
  bucket_size = '30s',
  initial_value = 7,
}
assert(
  replaced:sum() == 7,
  'redefining with a different shape should reset to the new initial value'
)

-- Sub-second bucket sizes are rounded up to the next whole second.
local sub = kumo.counter_series.define {
  name = 'test.subsecond',
  num_buckets = 3,
  bucket_size = '500ms',
}
sub:increment(5)
assert(sub:sum() == 5, 'sub-second bucket size should be accepted')
