local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

-- Confirm that text is loaded as a string
local text = kumo.secrets.load {
  key_data = 'hello',
}
utils.assert_eq(text, 'hello')

-- Confirm that binary can be loaded too
local text = kumo.secrets.load {
  key_data = '\x00',
}
utils.assert_eq(text, '\x00')

-- test error message for failure to load from a file
local ok, err = pcall(kumo.secrets.load, '/never ever exists')
assert(not ok, 'should fail')
utils.assert_matches(
  tostring(err),
  'KeySource failed to load data from file `/never ever exists`: No such file or directory'
)
