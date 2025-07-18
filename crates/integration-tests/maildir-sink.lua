-- This config acts as a sink that capture all received mail into a maildir
local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path
local utils = require 'policy-extras.policy_utils'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'

kumo.on('init', function()
  kumo.configure_accounting_db_path ':memory:'

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
  }

  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
  }

  kumo.define_spool {
    name = 'data',
    path = TEST_DIR .. '/data-spool',
  }

  kumo.define_spool {
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
  }
end)

local function apply_rejections(envelope_addr)
  if string.find(envelope_addr.user, 'tempfail') then
    kumo.reject(400, 'tempfail requested')
  end
  if string.find(envelope_addr.user, 'permfail') then
    kumo.reject(500, 'permfail requested')
  end
  if utils.starts_with(envelope_addr.user, '450-') then
    kumo.reject(450, 'you said ' .. envelope_addr.user)
  end
  if utils.starts_with(envelope_addr.user, '421-') then
    kumo.reject(421, 'disconnecting ' .. envelope_addr.user)
  end
  if utils.starts_with(envelope_addr.user, '550-') then
    kumo.reject(550, 'you said ' .. envelope_addr.user)
  end
end

kumo.on('smtp_server_rcpt_to', function(recipient)
  apply_rejections(recipient)
end)

kumo.on('smtp_server_mail_from', function(sender)
  apply_rejections(sender)
end)

kumo.on('smtp_server_message_received', function(msg)
  local sender = msg:sender().user
  if utils.starts_with(sender, 'disconnect-in-data-no-421') then
    kumo.disconnect(451, 'disconnecting ' .. sender, 'ForceDisconnect')
  end

  msg:set_meta('queue', 'maildir')
end)

kumo.on('get_queue_config', function(_domain, _tenant, _campaign)
  return kumo.make_queue_config {
    protocol = {
      maildir_path = TEST_DIR .. '/maildir',
    },
  }
end)

function simple_auth_check(user, password)
  local password_database = {
    ['daniel'] = 'tiger',
  }
  if password == '' then
    return false
  end
  return password_database[user] == password
end

kumo.on('http_server_validate_auth_basic', function(user, password)
  return simple_auth_check(user, password)
end)

kumo.on('smtp_server_auth_plain', function(authz, authc, password)
  print(
    string.format(
      "AUTH PLAIN: authz='%s' authc='%s' pass='%s'",
      authz,
      authc,
      password
    )
  )
  return simple_auth_check(authc, password)
end)
