local kumo = require 'kumo'

-- ./target/debug/kumod --policy test-client-cert.lua --user kumod
-- echo -e "quit\r\n" | nc -C localhost 25

-- Point to a server which can accept and log client side certificate for verification
-- After adding support for incoming client side cert, we get kumomta to accept the connection (put condition so we don't go into loop)
local mta = '10.0.0.1'

kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:25',
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:8000',
  }

  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumomta/data',
  }

  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumomta/meta',
  }
end)

kumo.on('smtp_server_connection_accepted', function(conn_meta)
  -- is there a way to run a function automatically after init completes ?
  inject()
end)

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  msg:set_meta('routing_domain', msg:recipient().domain)
  msg:set_meta('tenant', msg:recipient().domain)
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  return kumo.make_queue_config {
    egress_pool = domain,
    max_age = '60s',
    protocol = {
      smtp = {
        mx_list = { mta },
      },
    },
  }
end)

kumo.on('get_egress_pool', function(pool_name)
  return kumo.make_egress_pool {
    name = pool_name,
    entries = {
      { name = pool_name },
    },
  }
end)

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
  }
end)

kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    if routing_domain == 'openssl.cert.local' then
      kumo.log_info '### openssl'
      return kumo.make_egress_path {
        tls_prefer_openssl = true,
        enable_tls = 'OpportunisticInsecure',
        tls_certificate = '/tmp/kumomta/cert.pem',
        tls_private_key = '/tmp/kumomta/key.pem',
      }
    end
    if routing_domain == 'openssl.missing.local' then
      kumo.log_error '### openssl, this will give an error'
      return kumo.make_egress_path {
        tls_prefer_openssl = true,
        enable_tls = 'OpportunisticInsecure',
        tls_certificate = '/tmp/kumomta/cert.fake.pem',
        tls_private_key = '/tmp/kumomta/key.pem',
      }
    end
    if routing_domain == 'openssl.v3.local' then
      kumo.log_info '### openssl with v3'
      return kumo.make_egress_path {
        tls_prefer_openssl = true,
        enable_tls = 'OpportunisticInsecure',
        tls_certificate = '/tmp/kumomta/v3.crt',
        tls_private_key = '/tmp/kumomta/v3.key',
      }
    end
    if routing_domain == 'rustls.v1.local' then
      kumo.log_error '### rustls v1, this will give an error'
      return kumo.make_egress_path {
        tls_prefer_openssl = false,
        enable_tls = 'OpportunisticInsecure',
        tls_certificate = '/tmp/kumomta/cert.pem',
        tls_private_key = '/tmp/kumomta/key.pem',
      }
    end
    if routing_domain == 'rustls.local' then
      kumo.log_info '### rustls'
      return kumo.make_egress_path {
        tls_prefer_openssl = false,
        enable_tls = 'OpportunisticInsecure',
        tls_certificate = '/tmp/kumomta/v3.crt',
        tls_private_key = '/tmp/kumomta/v3.key',
      }
    end
    kumo.log_info '### default'
    return kumo.make_egress_path {
      enable_tls = 'OpportunisticInsecure',
      tls_prefer_openssl = true,
    }
  end
)

function inject()
  local recipients = {
    'user@rustls.local',
    'user@rustls.v1.local',
    'user@openssl.cert.local',
    'user@openssl.v3.local',
    'user@openssl.missing.local',
  }

  for _, rcpt in ipairs(recipients) do
    kumo.api.inject.inject_v1 {
      envelope_sender = '',
      content = 'Subject: testing\r\n\r\ntest',
      recipients = { { email = rcpt } },
    }
  end
end
