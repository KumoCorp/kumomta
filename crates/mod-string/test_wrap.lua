local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

utils.assert_eq(kumo.string.wrap 'hello', 'hello')

local long_string = string.rep('A', 200)
local expect_wrapped = string.rep('A', 100)
  .. '\r\n\t'
  .. string.rep('A', 100)
utils.assert_eq(kumo.string.wrap(long_string, 75, 100), expect_wrapped)

local long_string_spaced = string.rep(' hello there', 10)
utils.assert_eq(
  kumo.string.wrap(long_string_spaced, 75, 100),
  'hello there hello there hello there hello there hello there hello there\r\n'
    .. '\thello there hello there hello there hello there'
)
