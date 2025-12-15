local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'
local typing = require 'policy-extras.typing'

local Bool, Default, Record, List, Any, Option, String =
  typing.boolean,
  typing.default,
  typing.record,
  typing.list,
  typing.any,
  typing.option,
  typing.string

local MailAuthConfig = Record('MailAuthConfig', {
  dkim = Default(Bool, true),
  spf = Default(Bool, true),
  iprev = Default(Bool, true),
  smtp_auth = Default(Bool, true),
  dmarc = Default(Bool, true),
  arc = Default(Bool, true),

  add_auth_results_header = Default(Bool, true),
  server_id = Option(String),

  resolver = Option(Any),
})

-- It's not really Any, but a rust type that doesn't have a typing repr
local AuthenticationResult = Any

local MailAuthResult = Record('MailAuthResult', {
  -- List of AuthenticationResult, which doesn't having a typing repr
  dkim = List(AuthenticationResult),
  spf = Option(AuthenticationResult),
  iprev = Option(AuthenticationResult),
  smtp_auth = Option(AuthenticationResult),
  dmarc = Option(AuthenticationResult),
  arc = Option(AuthenticationResult),

  -- The overall set of authentication results
  auth_results = List(AuthenticationResult),
})

function mod.check(msg, config)
  local config = MailAuthConfig(config or {})

  local auth_results = {}
  local dkim_auth_results = {}

  local server_id = config.server_id or msg:get_meta 'hostname' or 'localhost'

  if config.dkim then
    dkim_auth_results = msg:dkim_verify(config.resolver)
    if #dkim_auth_results == 0 then
      dkim_auth_results = {
        {
          method = 'dkim',
          result = 'none',
          reason = 'message was not signed',
        },
      }
    end
    for _, ar in ipairs(dkim_auth_results) do
      table.insert(auth_results, ar)
    end
  end

  local spf_auth_result = nil
  if config.spf then
    local spf_disp = kumo.spf.check_msg(msg, config.resolver)
    spf_auth_result = spf_disp.result
    table.insert(auth_results, spf_disp.result)
  end

  local iprev_auth_result = nil
  if config.iprev then
    iprev_auth_result = mod.iprev_msg(msg, config.resolver)
    table.insert(auth_results, iprev_auth_result)
  end

  local smtp_auth_result = nil
  if config.smtp_auth then
    local auth_id = msg:get_meta 'authn_id'
    smtp_auth_result = {
      method = 'auth',
      result = 'none',
      props = {
        ['smtp.mailfrom'] = tostring(msg:sender()),
      },
    }
    if auth_id then
      smtp_auth_result.result = 'pass'
      smtp_auth_result.props['smtp.auth'] = auth_id
    end
    table.insert(auth_results, smtp_auth_result)
  end

  local dmarc_auth_result = nil
  if config.dmarc and config.dkim and config.spf then
    local dkim_auth_results = msg:dkim_verify(config.resolver)
    local spf_disp = kumo.spf.check_msg(msg, config.resolver)
    spf_auth_result = spf_disp.result

    local dmarc_disp = kumo.dmarc.check_msg(
      msg,
      false,
      dkim_auth_results,
      spf_auth_result,
      config.resolver
    )
    dmarc_auth_result = dmarc_disp.result

    table.insert(auth_results, dmarc_auth_result)
  end

  local arc_auth_result = nil
  if config.arc then
    arc_auth_result = msg:arc_verify(config.resolver)
    table.insert(auth_results, arc_auth_result)
  end

  if config.add_auth_results_header then
    msg:add_authentication_results(server_id, auth_results)
  end

  return MailAuthResult {
    dkim = dkim_auth_results,
    spf = spf_auth_result,
    iprev = iprev_auth_result,
    smtp_auth = smtp_auth_result,
    dmarc = dmarc_auth_result,
    arc = arc_auth_result,
    auth_results = auth_results,
  }
end

function mod.iprev_msg(msg, opt_resolver)
  local ip, port = utils.split_ip_port(msg:get_meta 'received_from')
  return mod.iprev(ip, opt_resolver)
end

function mod.iprev(ip, opt_resolver)
  -- https://datatracker.ietf.org/doc/html/rfc8601#autoid-28
  -- return an AuthenticationResult representing the status
  local result = {
    method = 'iprev',
    props = {
      ['smtp.remote-ip'] = ip,
    },
  }

  local ok, names = pcall(kumo.dns.lookup_ptr, ip, opt_resolver)
  if not ok then
    result.result = 'temperror'
    -- in the failed pcall case, `names` is really the error message
    result.reason =
      string.format('ip %s failed to resolve PTR: %s', ip, tostring(names))
    return result
  end

  if #names == 0 then
    result.result = 'permerror'
    result.reason = string.format('ip %s has no PTR', ip)
    return result
  end

  result.result = 'fail'
  for _, name in ipairs(names) do
    local ok, addrs = pcall(kumo.dns.lookup_addr, name, opt_resolver)
    if ok then
      if #addrs == 0 then
        result.reason = string.format('name %s has no A/AAAA', name)
      end
      for _, addr in ipairs(addrs) do
        if addr == ip then
          result.result = 'pass'
          result.reason = string.format('ip %s <-> %s', ip, name)
          return result
        end
      end
    else
      result.result = 'temperror'
      -- in the failed pcall case, `addrs` is really the error message
      result.reason =
        string.format('failed to resolve %s: %s', name, tostring(addrs))
    end
  end

  return result
end

function mod:test()
  kumo.dns.define_resolver('mail_auth.lua', {
    Test = {
      zones = {
        [[
$ORIGIN 0.0.127.in-addr.arpa.
1 30 IN PTR localhost.localdomain.
2 20 IN PTR borked.localdomain.
]],
        [[
$ORIGIN localdomain.
localhost 30 IN A 127.0.0.1
]],
      },
    },
  })

  local r = mod.iprev('127.0.0.1', 'mail_auth.lua')
  -- print(utils.dumps(r))
  utils.assert_eq(r.result, 'pass')
  utils.assert_eq(r.reason, 'ip 127.0.0.1 <-> localhost.localdomain.')

  local r = mod.iprev('30.0.0.1', 'mail_auth.lua')
  -- print(utils.dumps(r))
  utils.assert_eq(r.result, 'permerror')
  utils.assert_eq(r.reason, 'ip 30.0.0.1 has no PTR')

  local r = mod.iprev('127.0.0.2', 'mail_auth.lua')
  -- print(utils.dumps(r))
  utils.assert_eq(r.result, 'fail')
  utils.assert_eq(r.reason, 'name borked.localdomain. has no A/AAAA')

  local r = mod.iprev('10.0.0.1', 'mail_auth.lua')
  --  print(utils.dumps(r))
  utils.assert_eq(r.result, 'permerror')
  utils.assert_eq(r.reason, 'ip 10.0.0.1 has no PTR')

  local signer = kumo.dkim.rsa_sha256_signer {
    domain = 'example.com',
    selector = 's1',
    headers = { 'From', 'To' },
    key = 'example-private-dkim-key.pem',
  }

  local msg = kumo.make_message(
    'sender@example.com',
    'recip@example.com',
    'From: sender@example.com\r\nSubject: hello\r\n\r\nhello'
  )
  msg:set_meta('received_from', '127.0.0.1:42')
  local result = mod.check(msg, { resolver = 'mail_auth.lua' })
  -- print(utils.dumps(result))
  local ar = msg:get_first_named_header_value 'Authentication-Results'
  utils.assert_eq(
    ar,
    'localhost; dkim=none reason="message was not signed"; '
      .. 'spf=none reason="no SPF records found for example.com" '
      .. 'smtp.mailfrom=sender@example.com; iprev=pass '
      .. 'reason="ip 127.0.0.1 <-> localhost.localdomain." '
      .. 'smtp.remote-ip=127.0.0.1; auth=none '
      .. 'smtp.mailfrom=sender@example.com; dmarc=permerror '
      .. 'reason="no DMARC records found for example.com"; arc=none'
  )
end

return mod
