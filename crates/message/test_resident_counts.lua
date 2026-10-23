local kumo = require 'kumo'

local function data_count()
  local metrics = kumo.prometheus_metrics()
  local gauge = metrics.message_data_resident_count
  -- message_data_resident_count is absent from the metrics table until some
  -- message has made it resident. Treat that absence as a count of zero.
  return gauge and gauge.value or 0
end

local function new_msg(body)
  return kumo.make_message('sender@example.com', 'recip@example.com', body)
end

-- Hold references for the duration of the test to keep the messages, and their
-- counts, alive across the garbage collector.
local held = {}
local function keep(msg)
  table.insert(held, msg)
  return msg
end

local base = data_count()

-- message_data_resident_count must track exactly the messages whose body is
-- currently non-empty: constructing or assigning an empty body must leave it
-- unchanged, and only a non-empty body may increment it. #588
local empty = keep(new_msg '')
assert(
  data_count() == base,
  'constructing with an empty body must not increment the resident count'
)

-- A non-empty body is resident and counted.
keep(new_msg 'Subject: hi\r\n\r\nbody')
assert(
  data_count() == base + 1,
  'constructing with a non-empty body must increment the resident count'
)

-- Assigning an empty body leaves the message non-resident.
empty:set_data ''
assert(
  data_count() == base + 1,
  'assigning an empty body must not increment the resident count'
)

-- Assigning a non-empty body makes the previously-empty message resident.
empty:set_data 'Subject: hi\r\n\r\nbody'
assert(
  data_count() == base + 2,
  'assigning a non-empty body must increment the resident count'
)

print 'resident count tests OK'
