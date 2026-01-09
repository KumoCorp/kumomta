local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

kumo.on('get_acl_definition', function(resource)
  return nil
end)

kumo.on('main', function()
  local test_url = kumo.aaa.make_http_url_resource(
    '127.0.0.1:8080',
    'https://localhost/api/admin/baz'
  )

  local unauthenticated_auth_info = {}
  local trusted_auth_info = {
    groups = { 'kumomta:http-listener-trusted-ip' },
  }

  local query_result =
    kumo.aaa.query_resource_access(test_url, unauthenticated_auth_info, 'GET')
  utils.assert_eq(query_result.allow, false)
  utils.assert_eq(query_result.rule, nil)
  utils.assert_eq(query_result.resource, nil)

  local query_result =
    kumo.aaa.query_resource_access(test_url, trusted_auth_info, 'GET')
  utils.assert_eq(query_result.allow, true)
  utils.assert_eq(query_result.rule, {
    criteria = { Identity = { Group = 'kumomta:http-listener-trusted-ip' } },
    access = 'Allow',
    privilege = 'GET',
  })
  -- Enabled by the default /api/admin rule in assets/acls/default.toml
  utils.assert_eq(query_result.resource, 'http_listener/*/api/admin')
end)
