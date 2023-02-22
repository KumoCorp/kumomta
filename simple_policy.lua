local kumo = require 'kumo'

-- Called on startup to initialize the system
kumo.on('init', function()
  -- Define a listener.
  -- Can be used multiple times with different parameters to
  -- define multiple listeners!
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:2025',
    -- Override the hostname reported in the banner and other
    -- SMTP responses:
    -- hostname="mail.example.com",

    -- override the default set of relay hosts
    relay_hosts = { '127.0.0.1', '192.168.1.0/24' },

    -- Customize the banner.
    -- The configured hostname will be automatically
    -- prepended to this text.
    banner = 'Welcome to KumoMTA!',

    -- Unsafe! When set to true, don't save to spool
    -- at reception time.
    -- Saves IO but may cause you to lose messages
    -- if something happens to this server before
    -- the message is spooled.
    deferred_spool = false,
  }

  kumo.start_http_listener {
    listen = '0.0.0.0:8000',
  }

  -- Define the default "data" spool location; this is where
  -- message bodies will be stored
  --
  -- 'flush' can be set to true to cause fdatasync to be
  -- triggered after each store to the spool.
  -- The increased durability comes at the cost of throughput.
  --
  -- kind can be 'LocalDisk' (currently the default), 'Sled'
  -- or 'RocksDB'.
  --
  -- LocalDisk stores one file per message in a filesystem hierarchy.
  -- Sled is a key-value datastore.
  --
  -- Sled has 4x the throughput of LocalDisk when flush=false.
  -- Sled's throughput when flush=false is 3x its throughput when flush=true,
  -- making Sled a higher performing choice in both cases.
  -- Sled is considered beta by its authors, and upgrading to
  -- a newer version of Sled doesn't have a great story today,
  -- so you will need to evaluate which works best for your
  -- requirements.
  -- https://github.com/spacejam/sled#known-issues-warnings
  --
  -- RocksDB has >4x the throughput of LocalDisk, and enabling
  -- flush has a marginal (<10%) impact in early testing.
  kumo.define_spool {
    name = 'data',
    path = '/var/tmp/kumo-spool/data',
    flush = false,
    kind = 'RocksDB',
  }

  -- Define the default "meta" spool location; this is where
  -- message envelope and metadata will be stored
  kumo.define_spool {
    name = 'meta',
    path = '/var/tmp/kumo-spool/meta',
    flush = false,
    kind = 'RocksDB',
  }
end)

-- Called to validate the helo and/or ehlo domain
kumo.on('smtp_server_ehlo', function(domain)
  -- print('ehlo domain is', domain)
  -- Use kumo.reject to return an error to the EHLO command
  -- kumo.reject(420, 'wooooo!')
end)

-- Called to validate the sender
kumo.on('smtp_server_mail_from', function(sender)
  -- print('sender', tostring(sender))
  -- kumo.reject(420, 'wooooo!')
end)

-- Called to validate a recipient
kumo.on('smtp_server_mail_rcpt_to', function(rcpt)
  -- print('rcpt', tostring(rcpt))
end)

-- Called once the body has been received.
-- For multi-recipient mail, this is called for each recipient.
kumo.on('smtp_server_message_received', function(msg)
  -- print('id', msg:id(), 'sender', tostring(msg:sender()))

  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:sender().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    file_name = 'example-private-dkim-key.pem',
  }
  msg:dkim_sign(signer)

  -- set/get metadata fields
  -- msg:set_meta('foo', 'bar')
  -- print('meta foo is', msg:get_meta 'foo')
end)

-- Not the final form of this API, but this is currently how
-- we retrieve configuration used when making outbound
-- connections
kumo.on('get_site_config', function(domain, site_name)
  return kumo.make_site_config {
    enable_tls = 'OpportunisticInsecure',
  }
end)

-- Not the final form of this API, but this is currently how
-- we retrieve configuration used for managing a queue.
kumo.on('get_queue_config', function(queue_name)
  return kumo.make_queue_config {
    -- Age out messages after being in the queue for 2 minutes
    max_age = 120,
    retry_interval = 2,
    max_retry_interval = 8,
  }
end)
