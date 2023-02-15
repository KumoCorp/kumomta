local kumo = require 'kumo'

-- Called on startup to initialize the system
kumo.on('init', function()
  -- Define a listener.
  -- Can be used multiple times with different parameters to
  -- define multiple listeners!
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:2025',
    -- Override the hostname reported in the banner and other
    -- SMTP responses:
    -- hostname="mail.example.com",
  }

  -- Define the default "data" spool location; this is where
  -- message bodies will be stored
  kumo.define_spool {
    name = 'data',
    path = '/tmp/kumo-spool/data',
  }

  -- Define the default "meta" spool location; this is where
  -- message envelope and metadata will be stored
  kumo.define_spool {
    name = 'meta',
    path = '/tmp/kumo-spool/meta',
  }
end)

-- Called to validate the helo and/or ehlo domain
kumo.on('smtp_server_ehlo', function(domain)
  print('ehlo domain is', domain)
end)

-- Called to validate the sender
kumo.on('smtp_server_mail_from', function(sender)
  print('sender', tostring(sender))
  -- kumo.reject(420, 'wooooo!')
end)

-- Called to validate a recipient
kumo.on('smtp_server_mail_rcpt_to', function(rcpt)
  print('rcpt', tostring(rcpt))
end)

-- Called once the body has been received
kumo.on('smtp_server_message_received', function(msg)
  print('id', msg:id(), 'sender', tostring(msg:sender()))

  -- set/get metadata fields
  msg:set_meta('foo', 'bar')
  print('meta foo is', msg:get_meta 'foo')
end)
