local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

utils.assert_eq(kumo.string.starts_with('abc.hello', 'abc'), true)
utils.assert_eq(kumo.string.starts_with('abc.hello', 'hello'), false)
utils.assert_eq(kumo.string.starts_with('hello', 'hello'), true)
utils.assert_eq(kumo.string.starts_with('h1e2l3lo', 'hello'), false)

utils.assert_eq(kumo.string.ends_with('abc.hello', 'hello'), true)
utils.assert_eq(kumo.string.ends_with('abc.hello', 'abc'), false)
utils.assert_eq(kumo.string.ends_with('hello', 'hello'), true)
utils.assert_eq(kumo.string.ends_with('h1e2l3lo', 'hello'), false)
