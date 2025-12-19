local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local get_acl_map = kumo.memoize(function()
  return kumo.aaa.load_acl_map {
    key_data = [=[

[[acl."http_listener/*/api/admin/baz"]]
allow = true
privileges = ["GET"]
identity.Group = "kumomta:http-listener-trusted-ip"

    ]=],
  }
end, {
  name = 'acl_map_cache',
  ttl = '5 minutes',
  capacity = 4,
})

kumo.on('get_acl_definition', function(resource)
  return get_acl_map():get(resource)
end)

kumo.on('main', function()
  -- Turn off falling back to the default acl, so that only the acls returned
  -- from get_acl_definition are consulted and used.
  kumo.aaa.set_fall_back_to_acl_map(false)

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
  utils.assert_eq(query_result.resource, 'http_listener/*/api/admin/baz')

  local other_test_url = kumo.aaa.make_http_url_resource(
    '127.0.0.1:8080',
    'https://localhost/api/admin'
  )
  local query_result =
    kumo.aaa.query_resource_access(other_test_url, trusted_auth_info, 'GET')
  utils.assert_eq(query_result.allow, false)
  utils.assert_eq(query_result.rule, nil)
  utils.assert_eq(query_result.resource, nil)
end)
