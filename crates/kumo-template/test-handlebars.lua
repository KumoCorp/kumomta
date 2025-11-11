local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

-- "name or 'default'", sparkpost style, is not supported
local status, err = pcall(
  kumo.string.eval_template,
  'test_name_or_fred',
  [[{{ name or 'Fred' }}]],
  { name = 'John' },
  'Handlebars'
)
assert(not status, 'syntax is not supported')
utils.assert_matches(
  tostring(err),
  'Error rendering "test_name_or_fred" line 1, col 1: Helper not found name'
)

utils.assert_eq(
  kumo.string.eval_template(
    'something.html',
    [[{{{ name }}}]],
    { name = 'A&B' },
    'Handlebars'
  ),
  'A&B'
)

utils.assert_eq(
  kumo.string.eval_template(
    'something.html',
    [[{{{ titleCase name }}}]],
    { name = 'fred' },
    'Handlebars'
  ),
  'Fred'
)
