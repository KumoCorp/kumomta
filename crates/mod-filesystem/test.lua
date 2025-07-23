local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'
print 'doing filesystem tests'

local filename = os.tmpname()

local writer = kumo.fs.open(filename, 'w+')
local reader = kumo.fs.open(filename, 'r')

writer:write 'hello'
utils.assert_eq(reader:read(), '') -- no data visible until flushed
writer:write ' world'
writer:flush()
utils.assert_eq(reader:read(), 'hello world')

local new_pos = writer:seek 'set'
utils.assert_eq(new_pos, 0)
writer:write 'woot'
writer:seek 'set' -- check that seek implicitly flushes
utils.assert_eq(reader:read(), '')

reader:seek('set', 0)
utils.assert_eq(reader:read(4), 'woot')
reader:seek(4)
utils.assert_eq(reader:read(4), 'rld')
reader:seek('cur', -4)
utils.assert_eq(reader:read(4), 'orld')
reader:seek 'end'
utils.assert_eq(reader:read(), '')

assert(
  not pcall(reader.seek, reader, 'bogus'),
  'seek with bogus whence should error'
)

reader:close()
assert(not pcall(reader.read, reader), 'read after close should error')

writer:close()
assert(
  not pcall(writer.write, writer, 'foo'),
  'write after close should error'
)
