local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

utils.assert_eq(
  kumo.string.eval_template('example.json', [[{{ name }}]], { name = 'John' }),
  [["John"]]
)

utils.assert_eq(
  kumo.string.eval_template(
    'example.html',
    [[{{ name }}]],
    { name = 'John & Co' }
  ),
  [[John &amp; Co]]
)

utils.assert_eq(
  kumo.string.eval_template(
    'example.txt',
    [[{{ name }}]],
    { name = 'John & Co' }
  ),
  [[John & Co]]
)

utils.assert_eq(
  kumo.string.eval_template(
    'example.txt',
    [[{{ name }}]],
    { name = 'John & Co' }
  ),
  [[John & Co]],
  'Jinja'
)
