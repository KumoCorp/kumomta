local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

kumo.audit_series.define('falure', {
  bucket_count = 3,
  window = 300,
})
