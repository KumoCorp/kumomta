local kumo = require 'kumo'

-- This file exercises the various resolver setup functions
-- with a variety of configurations

-- Explicitly use hickory with the system resolver.
-- This is the default configuration and doesn't need
-- to be made explicit in your config
kumo.dns.configure_resolver 'HickorySystemConfig'

-- Use hickory with custom upstream dns
kumo.dns.configure_resolver {
  Hickory = {
    name_servers = {
      '10.0.0.1:53',
    },
  },
}

-- Use hickory with the detailed name-server form, exercising each
-- accepted value of `protocol` and a custom bind address.
kumo.dns.configure_resolver {
  Hickory = {
    name_servers = {
      { socket_addr = '10.0.0.1:53', protocol = 'udp' },
      { socket_addr = '10.0.0.1:53', protocol = 'tcp' },
      {
        socket_addr = '10.0.0.2:53',
        protocol = 'udp_then_tcp',
        trust_negative_responses = false,
        bind_addr = '0.0.0.0:0',
      },
    },
  },
}

-- Use Unbound with custom upstream dns
kumo.dns.configure_resolver {
  Unbound = {
    name_servers = {
      '10.0.0.1:53',
    },
  },
}

-- Use a test resolver with a static, explicitly limited configuration
kumo.dns.configure_resolver {
  Test = {
    zones = {
      [[
$ORIGIN 0.0.127.in-addr.arpa.
1 30 IN PTR localhost.
  ]],
    },
  },
}

-- Aggregate a test resolver with the system resolver; the test
-- resolver takes precedence over the system resolver.
kumo.dns.configure_resolver {
  Aggregate = {
    Test = {
      zones = {
        [[
$ORIGIN 0.0.127.in-addr.arpa.
1 30 IN PTR localhost.
  ]],
      },
    },
    'HickorySystemConfig',
  },
}
