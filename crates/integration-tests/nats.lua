local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local ADDRESS = os.getenv 'ADDRESS'
local SUBJECT = os.getenv 'SUBJECT'
local USERNAME = os.getenv 'USERNAME'
local PASSWORD = os.getenv 'PASSWORD'

kumo.on('init', function()
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

  local producer = kumo.nats.connect {
    servers = { ADDRESS },
    name = "nats-client",
    auth = {
      password = { key_data = PASSWORD },
      username = { key_data = USERNAME },
    },
  }

  producer:publish {
    subject = SUBJECT,
    payload = "payload-1",
    await_ack = true
  }

  producer:publish {
    subject = SUBJECT,
    payload = "payload-2",
    await_ack = false
  }

  producer:publish {
    subject = SUBJECT,
    payload = "payload-3",
    headers = {
      ["Nats-Msg-Id"] = "unique-key-3",
    },
    await_ack = true
  }

  producer:close()

  producer:publish {
    subject = SUBJECT,
    payload = "not sent because already closed",
  }
end)
