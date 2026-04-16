local kumo = require 'kumo'
package.path = 'assets/?.lua;' .. package.path
local utils = require 'policy-extras.policy_utils'

kumo.on('loader-example-1', function(one, two)
  return string.format('called-with-%s-%s', one, two)
end)

kumo.on('main', function()
  local text = kumo.secrets.load {
    event_name = 'loader-example-1',
    event_args = { 'a', 'b' },
  }
  utils.assert_eq(text, 'called-with-a-b')

  local text = kumo.secrets.load {
    event_name = 'loader-example-1',
    event_args = { 1, 'b' },
  }
  utils.assert_eq(text, 'called-with-1-b')
end)
