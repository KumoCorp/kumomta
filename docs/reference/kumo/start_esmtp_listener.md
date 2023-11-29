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

## data_buffer_size

Specified the size of the buffer used to read chunks of the message payload
during the `DATA` phase of the SMTP transaction.  Making this larger will
improve the throughput in terms of bytes-per-syscall at the expense of
using more RAM.

The default size is 128KB (`128 * 1024`).  If your average message size is
significantly larger than the default, then you may wish to increase this
value.

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

## hostname

Specifies the hostname to report in the banner and other SMTP responses.
The default, if unspecified, is to use the hostname of the local machine.

```lua
kumo.start_esmtp_listener {
  -- ..
  hostname = 'mail.example.com',
}
```

## invalid_line_endings

{{since('2023.11.28-b5252a41', indent=True)}}
    Specifies the behavior when the received DATA contains invalid line
    endings.  The SMTP protocol requires that each line of the DATA be
    separated by canonical CRLF sequences. Immediately after receiving the DATA
    payload, but before any other policy events are triggered, if the received
    DATA is non-conforming the value of this parameter is checked to determine
    what to do. It has three possible values:

    * `"Deny"` - this is the default. The incoming message will be
      rejected.
    * `"Allow"` - The incoming message will be accepted. Depending
      on the configured policy, some later policy actions may fail
      to parse the message, and DKIM signatures may be created that
      are not possible to validate correctly.  There is no guarantee
      that any resulting message will be routable to its intended
      destination.
    * `"Fix"` - the line endings will be normalized to CRLF and the
      message will be accepted.  It's possible for this to invalidate
      any signatures that may have already been present in the message.

## line_length_hard_limit

{{since('2023.11.28-b5252a41', indent=True)}}
    The SMTP protocol specification defines the maximum length of a line in the
    protocol.  The limit exists because there are SMTP implementations that are
    simply not capable of reading longer lines.

    This option sets the limit on line length that is enforced by KumoMTA. The
    default matches the RFC specified limit of `998`.  When the line length
    limit is exceeded, KumoMTA will return a "line too long" error to the
    client.

    You can raise this limit, but doing so may allow messages to be accepted
    that will be unable to be relayed to other SMTP implementations.

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

Specifies the maximum number of consecutive `MAIL FROM` commands that can be
issued for a given SMTP connection.  When the limit is reached, transient
failures will be returned to those additional `MAIL FROM` commands.

```lua
kumo.start_esmtp_listener {
  max_messages_per_connection = 10000,
}
```

## max_message_size

Specifies the maximum size of a message that can be relayed through
this listener, in bytes.

The default is `20 MB` (`20 * 1024 * 1024`).

Messages exceeding this size will be rejected.

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
Each item can be an IP literal or a CIDR mask. **Note** that the CIDR notation 
is strict, so that 192.168.1.0/24 is valid but 192.168.1.1/24 is not because 
that final octet isn’t valid in a /24.


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

