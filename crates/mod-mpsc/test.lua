local kumo = require 'kumo'
local mpsc = kumo.mpsc

print 'mpsc tests'

local Q = mpsc.define 'mod-mpsc-test'
print('defined as', Q)

local Q_ALIAS = mpsc.define 'mod-mpsc-test'

local redefined = mpsc.define('mod-mpsc-test', 128)
assert(true, 're-defining a queue with different params is silently accepted')

Q:send 'hello'
Q:try_send 'there'

local first = Q:recv()
assert(first == 'hello', first)

-- Verify that we receive in the correct order via the alias
local second = Q_ALIAS:recv()
assert(second == 'there', second)

assert(not Q:try_recv(), 'nothing to receive right now')
assert(Q:is_empty(), 'nothing to receive right now')
assert(Q:len() == 0, 'nothing to receive right now')

for i = 1, 10 do
  Q:send(i)
end

local batch = Q:recv_many(128)
assert(#batch == 10, 'got batch of 10 items')
for i = 1, 10 do
  assert(
    batch[i] == i,
    string.format('item %d sould be itself, but is %s', i, batch[i])
  )
end

Q:send_timeout('woot', 0.5)
assert(Q:recv() == 'woot', 'send timeout works on unbounded channel')

local BOUNDED = mpsc.define('mod-mpsc-test-bounded', 1)
assert(BOUNDED:try_send(1))
assert(not BOUNDED:try_send(2), 'should not fit second item')

local ok, err = pcall(BOUNDED.send_timeout, BOUNDED, 'foo', 0.05)
assert(
  not ok,
  'should not be able to send within 0.05 seconds because queue is full'
)

assert(BOUNDED:try_recv() == 1)
assert(BOUNDED:try_send(2), 'should NOW fit second item')
assert(BOUNDED:try_recv() == 2)
assert(BOUNDED:is_empty(), 'should currently be empty')
assert(BOUNDED:len() == 0, 'should currently be empty')

BOUNDED:send_timeout('woot', 0.5)
assert(BOUNDED:recv() == 'woot', 'send timeout works on bounded channel')

-- Queues are not memoizable, so not compatible
local ok, err = pcall(Q.send, Q, Q)
assert(not ok, err)

-- cidrmap is memoizable, can be passed through Q
local object = kumo.cidr.make_map { ['127.0.0.1'] = 42 }
print(object)
Q:send(object)
local passed_object = Q:recv()
print('got object', passed_object)
assert(passed_object['127.0.0.1'] == 42, 'map operates correctly')

print 'mpsc tests done!'
