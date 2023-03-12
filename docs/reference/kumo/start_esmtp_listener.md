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

## client_timeout

Controls the timeout used when reading data from the client.
If no data arrives within the specified timeout, the server
will close the connection to the client.

```lua
kumo.start_esmtp_listener {
  -- The default is 1 minute
  client_timeout = '1 minute',
}
```

## deferred_spool

!!! danger
    Enabling this option may result in loss of accountability for messages.
    You should satisfy yourself that your system is able to recognize and
    deal with that scenario if/when it arises.

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
      -- allow relaying mail from anyone, so long as it is
      -- addressed to example.com
      relay_to = true,
    },
    ['bounce.example.com'] = {
      -- accept and log OOB bounce reports sent to bounce.example.com
      log_oob = true,
    },
    ['fbl.example.com'] = {
      -- accept and log ARF feedback reports sent to fbl.example.com
      log_arf = true,
    },
    ['send.example.com'] = {
      -- relay to anywhere, so long as the sender domain is send.example.com
      -- and the connected peer matches one of the listed CIDR blocks
      relay_from = { '10.0.0.0/24' },
    },
    -- wildcards are permitted. This will match
    -- <anything>.example.com that doesn't have
    -- another non-wildcard entry explicitly
    -- listed in this set of domains.
    -- Note that "example.com" won't match
    -- "*.example.com".
    ['*.example.com'] = {
      -- You can specify multiple options if you wish
      log_oob = true,
      log_arf = true,
      relay_to = true,
    },
    -- and you can explicitly set options to false to
    -- essentially exclude an entry from a wildcard
    ['www.example.com'] = {
      relay_to = false,
      log_arf = false,
      log_oob = false,
    },
  },
}
```

When the SMTP `RCPT TO` command is issued by the client, the destination
domain is resolved from this domain configuration.

If none of `relay`, `oob` or `arf` are set to true, the `RCPT TO` command
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

You may specify that the certificate be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.start_esmtp_listener {
  -- ..
  tls_certificate = {
    vault_mount = 'secret',
    vault_path = 'tls/mail.example.com.cert',

    -- Specify how to reach the vault; if you omit these,
    -- values will be read from $VAULT_ADDR and $VAULT_TOKEN

    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"
  },
}
```

The key must be stored as `key` (even though this is a certificate!) under the
`path` specified.  For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com.cert key=@mail.example.com.cert
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

You may specify that the key be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.start_esmtp_listener {
  -- ..
  tls_private_key = {
    vault_mount = 'secret',
    vault_path = 'tls/mail.example.com.key',

    -- Specify how to reach the vault; if you omit these,
    -- values will be read from $VAULT_ADDR and $VAULT_TOKEN

    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"
  },
}
```

The key must be stored as `key` under the `path` specified.
For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com key=@mail.example.com.key
```

## trace_headers

Controls the addition of tracing headers to received messages.

KumoMTA can add two different headers to aid in later tracing:

* The standard `"Received"` header which captures SMTP relay hops on their path to the inbox
* A supplemental header which can be used to match feedback reports back to the
  originating mailing

Prior to triggering the
[smtp_server_message_received](../events/smtp_server_message_received.md)
event the standard `"Received"` header will be added to the
message.  Then, once the event completes and your policy has had the
opportunity to alter the meta data associated with the message, the
supplemental header will be added.

```lua
kumo.start_esmtp_listener {
  -- ..
  trace_headers = {
    -- this is the default: add the Received: header
    received_header = true,

    -- this is the default: add the supplemental header
    supplemental_header = true,

    -- this is the default: the name of the supplemental header
    header_name = 'X-KumoRef',

    -- names of additional meta data fields
    -- to include in the header. TAKE CARE! The header will be
    -- base64 encoded to prevent casual introspection, but the
    -- header is NOT encrypted and the values of the meta data
    -- fields included here should be considered to be public.
    -- The default is not to add any meta data fields, but you
    -- might consider setting something like:
    -- include_meta_names = { 'tenant', 'campaign' },
    include_meta_names = {},
  },
}
```

Here's an example of a supplemental header from a message:

```
X-KumoRef: eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoidGVzdEBleGFtcGxlLmNvbSJ9
```

the decoded payload contains a magic marker key as well as the recipient of the
original message:

```json
{"_@_":"\\_/","recipient":"test@example.com"}
```

Any meta data fields that were listed in `include_meta_names`, if the corresponding
meta data was set in the message, would also be captured in the decoded payload.

KumoMTA will automatically extract this supplemental trace header information
from any `X-` header that is successfully parsed and has the magic marker key
when processing the original message payload of an incoming ARF report.

