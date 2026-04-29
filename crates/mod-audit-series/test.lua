local kumo = require 'kumo'

-- Confirm that initial value is set correctly
local initial = kumo.audit_series.define('test.initial', 6, '60s', 10)

assert(initial:sum() == 10, 'sum after initial value should be 10')

-- define(name, num_buckets, bucket_size_seconds, initial_value)
local audit = kumo.audit_series.define('test.audit', 6, '1m')

-- Calling define with the same name should return the existing cached series.
local same = kumo.audit_series.define('test.audit', 6, '60s')

audit:increment(2)
audit:delta(-1)
assert(audit:sum() == 1, 'sum after increment+delta should be 1')
assert(same:sum() == 1, 'second handle should see same cached value')

audit:observe(10)

assert(audit:sum() == 10, 'sum after observe should be 10')
assert(
  audit:sum_over '2m' == 10,
  'sum_over should include current bucket value'
)
