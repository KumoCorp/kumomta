# `kumo.start_esmtp_listener {PARAMS}`

Configure and start ESMTP service.

This function should be called only from inside your [init](../events/init.md)
event handler.

`PARAMS` is a lua table that can accept the keys listed below.

To listen on multiple IP/port combinations, simply call
`kump.start_esmtp_listener` multiple times with the appropriate parameters.

```lua
kumo.on('init', function()
  -- use the same settings for ports 25 and 2026, without repeating them all
  for _, port in ipairs { 25, 2026 } do
    kumo.start_esmtp_listener {
      listen = '0:' .. tostring(port),
      relay_hosts = { '0.0.0.0/0' },
    }
  end
end)
```

## banner

Customize the banner that is returned to clients when they first connect.
The configured hostname will be automatically prepended to this text, so
you should not include a hostname.

```lua
kumo.start_esmtp_listener {
  -- ..
  banner = 'Welcome to KumoMTA!',
}
```

## deferred_spool

```admonish danger
Enabling this option may result in loss of accountability for messages.
You should satisfy yourself that your system is able to recognize and
deal with that scenario if/when it arises.
```

When set to `true`, incoming messages are retained in memory until after
their first transient delivery failure.

This can have a dramatic impact on throughput by removing local storage I/O as
a bottleneck, but introduces a risk of forgetting about those messages if the
machine loses power or if the **kumod** process exits unexpectedly.

```lua
kumo.start_esmtp_listener {
  -- ..
  deferred_spool = false,
}
```

## domains

By default, unless the client is connecting from one of the `relay_hosts`,
relaying is denied.

You can specify relaying options on a per-domain basis via the `domains`
configuration:

```lua
kumo.start_esmtp_listener {
  -- ...
  domains = {
    ['example.com'] = {
      relay = true,
    },
    ['bounce.example.com'] = {
      oob = true,
    },
    ['fbl.example.com'] = {
      fbl = true,
    },
    -- wildcards are permitted. This will match
    -- <anything>.example.com that doesn't have
    -- another non-wildcard entry explicitly
    -- listed in this set of domains.
    -- Note that "example.com" won't match
    -- "*.example.com".
    ['*.example.com'] = {
      -- You can specify multiple options if you wish
      oob = true,
      fbl = true,
      relay = true,
    },
    -- and you can explicitly set options to false to
    -- essentially exclude an entry from a wildcard
    ['www.example.com'] = {
      relay = false,
      fbl = false,
      oob = false,
    },
  },
}
```

When the SMTP `RCPT TO` command is issued by the client, the destination
domain is resolved from this domain configuration.

If none of `relay`, `oob` or `fbl` are set to true, the `RCPT TO` command
is rejected.

Once the `DATA` stage has transmitted the message content, and after the
[smtp_server_message_received](../events/smtp_server_message_received.md) event
has been processed, and the reception logged (which is where OOB and FBL data
is parsed and logged), the recipient domain is resolved from the domain list
again; if `relay` is `false` then the message will not be spooled and that will
be the end of its processing.

## hostname

Specifies the hostname to report in the banner and other SMTP responses.
The default, if unspecified, is to use the hostname of the local machine.

```lua
kumo.start_esmtp_listener {
  -- ..
  hostname = 'mail.example.com',
}
```


## listen

Specifies the local IP and port number to which the ESMTP service
should bind and listen.

Use `0.0.0.0` to bind to all IPv4 addresses.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
}
```

## max_messages_per_connection

Specified the maximum number of consecutive `MAIL FROM` commands that can be
issued for a given SMTP connection.  When the limit is reached, transient
failures will be returned to those additional `MAIL FROM` commands.

```lua
kumo.start_esmtp_listener {
  max_messages_per_connection = 10000,
}
```

## max_recipients_per_message

Specifies the maximum number of consecutive `RCPT TO` commands that can be
issued for a given SMTP transaction.  When the limit is reached, transient
failures will be returned to those additional `RCPT TO` commands.

```lua
kumo.start_esmtp_listener {
  max_recipients_per_message = 1024,
}
```

## relay_hosts

Specify the hosts which are allowed to relay email via this ESMTP service.
Each item can be an IP literal or a CIDR mask.

The defaults are to allow relaying only from the local host:

```lua
kumo.start_esmtp_listener {
  -- ..
  relay_hosts = { '127.0.0.1', '::1' },
}
```

## tls_certificate

Specify the path to a TLS certificate file to use for the server identity when
the client issues `STARTTLS`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

```lua
kumo.start_esmtp_listener {
  -- ..
  tls_certificate = '/path/to/cert.pem',
}
```

## tls_private_key

Specify the path to the TLS private key file that corresponds to the `tls_certificate`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

```lua
kumo.start_esmtp_listener {
  -- ..
  tls_private_key = '/path/to/key.pem',
}
```


