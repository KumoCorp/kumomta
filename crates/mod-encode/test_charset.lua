local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

utils.assert_eq(kumo.encode.charset_decode('iso-8859-1', '\xa9'), '©')
utils.assert_eq(kumo.encode.charset_encode('iso-8859-1', '©'), '\xa9')
