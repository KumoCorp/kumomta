local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

utils.assert_eq(
  kumo.string.eval_template(
    'something.txt',
    [[{{ response | normalize_smtp_response }}]],
    { response = '250 OK ids=8a5475ccbbc611eda12250ebf67f93bd' },
    'Jinja'
  ),
  '250 OK ids={uuid}'
)
